#!/usr/bin/env node
// Version management for Local Dictation (SemVer 2.0.0).
//
// Source of truth: [workspace.package].version in Cargo.toml. Every crate
// inherits it, and Tauri reads it when tauri.conf.json has no "version".
// package.json mirrors it. CHANGELOG.md must have a matching release entry.
//
//   node scripts/version.mjs check
//   node scripts/version.mjs bump <major|minor|patch|X.Y.Z[-pre]> [--date YYYY-MM-DD]
//
// `bump` edits files and Cargo.lock only. It never commits, tags or pushes.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const path = (p) => join(root, p);
const read = (p) => readFileSync(path(p), "utf8");

// SemVer 2.0.0 (build metadata deliberately unsupported: it is ignored for precedence).
const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?$/;
const WS_VERSION = /(\[workspace\.package\][^[]*?\nversion = ")([^"]+)(")/;
const REPO = "https://github.com/cahya-wirawan/dictation-app";

function fail(msg) {
  console.error(`version: ${msg}`);
  process.exit(1);
}

function cargoVersion() {
  const m = read("Cargo.toml").match(WS_VERSION);
  if (!m) fail("no version in [workspace.package] of Cargo.toml");
  return m[2];
}

function packageVersion() {
  return JSON.parse(read("package.json")).version;
}

function workspaceMembers() {
  const out = execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps", "--offline"], {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  });
  return JSON.parse(out).packages.map((p) => ({ name: p.name, version: p.version }));
}

function lockVersions(names) {
  const lock = read("Cargo.lock");
  const found = new Map();
  for (const block of lock.split("[[package]]")) {
    const name = block.match(/\nname = "([^"]+)"/)?.[1];
    const version = block.match(/\nversion = "([^"]+)"/)?.[1];
    if (name && names.has(name) && !block.includes("\nsource = ")) found.set(name, version);
  }
  return found;
}

function check() {
  const errors = [];
  const v = cargoVersion();
  if (!SEMVER.test(v)) errors.push(`Cargo.toml version "${v}" is not valid SemVer`);
  const pkg = packageVersion();
  if (pkg !== v) errors.push(`package.json version ${pkg} != Cargo.toml ${v}`);
  if (/"version"\s*:/.test(read("src-tauri/tauri.conf.json")))
    errors.push(`src-tauri/tauri.conf.json must not set "version" (Tauri inherits Cargo's)`);

  const members = workspaceMembers();
  for (const m of members) if (m.version !== v) errors.push(`crate ${m.name} is ${m.version}, expected ${v}`);
  const locked = lockVersions(new Set(members.map((m) => m.name)));
  for (const m of members) {
    const lv = locked.get(m.name);
    if (lv !== v) errors.push(`Cargo.lock has ${m.name} ${lv ?? "missing"}, expected ${v} (run cargo update --workspace)`);
  }

  const changelog = read("CHANGELOG.md");
  if (!changelog.includes("## [Unreleased]")) errors.push("CHANGELOG.md has no ## [Unreleased] section");
  if (!new RegExp(`^## \\[${v.replace(/\./g, "\\.")}\\] - \\d{4}-\\d{2}-\\d{2}$`, "m").test(changelog))
    errors.push(`CHANGELOG.md has no "## [${v}] - YYYY-MM-DD" entry`);

  if (errors.length) fail(`inconsistent version state:\n  - ${errors.join("\n  - ")}`);
  console.log(`version ${v}: consistent (Cargo workspace, ${members.length} crates, Cargo.lock, package.json, Tauri, CHANGELOG)`);
}

function nextVersion(current, spec) {
  if (SEMVER.test(spec)) return spec;
  const m = current.match(SEMVER);
  if (!m) fail(`current version ${current} is not SemVer`);
  let [major, minor, patch] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const pre = m[4] !== undefined;
  switch (spec) {
    case "major":
      // 1.0.0-rc.1 → 1.0.0; otherwise increment.
      return pre && minor === 0 && patch === 0 ? `${major}.0.0` : `${major + 1}.0.0`;
    case "minor":
      return pre && patch === 0 ? `${major}.${minor}.0` : `${major}.${minor + 1}.0`;
    case "patch":
      return pre ? `${major}.${minor}.${patch}` : `${major}.${minor}.${patch + 1}`;
    default:
      return fail(`bump expects major|minor|patch|X.Y.Z, got "${spec}"`);
  }
}

function compare(a, b) {
  const pa = a.match(SEMVER);
  const pb = b.match(SEMVER);
  for (let i = 1; i <= 3; i++) if (Number(pa[i]) !== Number(pb[i])) return Number(pa[i]) - Number(pb[i]);
  if (pa[4] === pb[4]) return 0;
  if (pa[4] === undefined) return 1; // release > pre-release
  if (pb[4] === undefined) return -1;
  const xa = pa[4].split(".");
  const xb = pb[4].split(".");
  for (let i = 0; i < Math.max(xa.length, xb.length); i++) {
    if (xa[i] === undefined) return -1;
    if (xb[i] === undefined) return 1;
    const na = /^\d+$/.test(xa[i]);
    const nb = /^\d+$/.test(xb[i]);
    if (na && nb && Number(xa[i]) !== Number(xb[i])) return Number(xa[i]) - Number(xb[i]);
    if (na !== nb) return na ? -1 : 1;
    if (xa[i] !== xb[i]) return xa[i] < xb[i] ? -1 : 1;
  }
  return 0;
}

function bump(spec, date) {
  if (!spec) fail("usage: version.mjs bump <major|minor|patch|X.Y.Z> [--date YYYY-MM-DD]");
  check(); // never bump from an inconsistent state
  const current = cargoVersion();
  const next = nextVersion(current, spec);
  if (compare(next, current) <= 0) fail(`${next} is not greater than ${current}`);
  if (date && !/^\d{4}-\d{2}-\d{2}$/.test(date)) fail(`--date must be YYYY-MM-DD`);
  const day = date ?? new Date().toISOString().slice(0, 10);

  let changelog = read("CHANGELOG.md");
  const unreleased = changelog.match(/## \[Unreleased\]\n([\s\S]*?)(?=\n## \[)/);
  if (!unreleased || !/^### /m.test(unreleased[1]))
    fail("CHANGELOG.md [Unreleased] has no entries; describe the changes before releasing");
  changelog = changelog.replace("## [Unreleased]\n", `## [Unreleased]\n\n## [${next}] - ${day}\n`);
  changelog = changelog.replace(
    /^\[Unreleased\]: .*$/m,
    `[Unreleased]: ${REPO}/compare/v${next}...HEAD\n[${next}]: ${REPO}/compare/v${current}...v${next}`,
  );
  writeFileSync(path("CHANGELOG.md"), changelog);

  writeFileSync(path("Cargo.toml"), read("Cargo.toml").replace(WS_VERSION, `$1${next}$3`));
  const pkg = JSON.parse(read("package.json"));
  pkg.version = next;
  writeFileSync(path("package.json"), `${JSON.stringify(pkg, null, 2)}\n`);
  execFileSync("cargo", ["update", "--workspace", "--offline"], { cwd: root, stdio: "inherit" });

  check();
  console.log(`\nBumped ${current} → ${next}. Review, then:\n  git commit -am "Release v${next}"\n  git tag -a v${next} -m "v${next}"`);
}

const [cmd, arg, ...rest] = process.argv.slice(2);
const dateIdx = rest.indexOf("--date");
if (cmd === "check") check();
else if (cmd === "bump") bump(arg, dateIdx >= 0 ? rest[dateIdx + 1] : undefined);
else fail("usage: version.mjs <check|bump> ...");
