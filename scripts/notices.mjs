#!/usr/bin/env node
// Third-party notices, license policy and SBOM for what actually ships
// (docs/10 §2): the app (src-tauri) and the worker, their *normal* Rust
// dependencies on the release targets, the production npm packages
// bundled into the UI, whisper.cpp/ggml (vendored C/C++) and the model weights.
//
//   node scripts/notices.mjs            write THIRD_PARTY_NOTICES.md
//   node scripts/notices.mjs --check    fail if it is stale or a license is not allowed
//   node scripts/notices.mjs --sbom F   also write a CycloneDX 1.5 JSON SBOM to F
//
// Output is deterministic (sorted, no timestamps) so --check can diff it.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(root, "THIRD_PARTY_NOTICES.md");
const TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"];
const SHIPPED_ROOTS = ["dictation-app", "transcription-worker"];
// Permissive licenses acceptable in a distributed binary. Anything else
// (GPL family, unknown, missing) fails --check and needs a human decision.
const ALLOWED = new Set([
  "MIT", "MIT-0", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause",
  "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016", "MPL-2.0", "CC0-1.0", "BSL-1.0", "0BSD",
  "Unlicense", "CDLA-Permissive-2.0", "NCSA",
]);

const args = process.argv.slice(2);
const check = args.includes("--check");
const sbomAt = args.includes("--sbom") ? args[args.indexOf("--sbom") + 1] : null;

/** "A OR (B AND C)"-style SPDX; allowed if any OR branch has only allowed terms. */
function allowed(expr) {
  if (!expr) return false;
  const norm = expr.replace(/\//g, " OR ").replace(/[()]/g, " ").replace(/\s+/g, " ").trim();
  return norm.split(" OR ").some((branch) =>
    branch.split(" AND ").every((t) => ALLOWED.has(t.trim())),
  );
}

function licenseTexts(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => /^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT)/i.test(f))
    .sort()
    .map((f) => {
      try {
        return { file: f, text: readFileSync(join(dir, f), "utf8").replace(/\r\n/g, "\n").trim() };
      } catch {
        return null;
      }
    })
    .filter(Boolean);
}

// ---------- Rust ----------
const meta = JSON.parse(
  execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--locked", ...TARGETS.flatMap((t) => ["--filter-platform", t])],
    { cwd: root, maxBuffer: 256 * 1024 * 1024 },
  ).toString(),
);
const byId = new Map(meta.packages.map((p) => [p.id, p]));
const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
const workspace = new Set(meta.workspace_members);
const seen = new Set();
const stack = meta.packages.filter((p) => SHIPPED_ROOTS.includes(p.name) && workspace.has(p.id)).map((p) => p.id);
while (stack.length) {
  const id = stack.pop();
  if (seen.has(id)) continue;
  seen.add(id);
  for (const d of nodes.get(id)?.deps ?? []) {
    // Only code linked into the binaries: skip dev- and build-only edges.
    if (d.dep_kinds.some((k) => k.kind === null)) stack.push(d.pkg);
  }
}
const crates = [...seen]
  .filter((id) => !workspace.has(id))
  .map((id) => byId.get(id))
  .map((p) => ({
    ecosystem: "cargo",
    name: p.name,
    version: p.version,
    license: p.license ?? (p.license_file ? `see ${p.license_file}` : null),
    source: p.repository ?? `https://crates.io/crates/${p.name}`,
    texts: licenseTexts(dirname(p.manifest_path)),
  }));

// ---------- npm (bundled into the UI) ----------
const npmJson = JSON.parse(
  execFileSync("pnpm", ["licenses", "list", "--prod", "--json"], { cwd: root, maxBuffer: 64 * 1024 * 1024 }).toString(),
);
const npm = Object.values(npmJson)
  .flat()
  .flatMap((p) =>
    p.versions.map((v, i) => ({
      ecosystem: "npm",
      name: p.name,
      version: v,
      license: p.license,
      source: p.homepage ?? `https://www.npmjs.com/package/${p.name}`,
      texts: licenseTexts(p.paths[i] ?? p.paths[0]),
    })),
  );

// ---------- vendored native code and model weights ----------
const whisperDir = join(root, "vendor/whisper.cpp");
const native = [
  {
    ecosystem: "native",
    name: "whisper.cpp (incl. ggml)",
    version: "1.9.5 (d1be6fde11ac6e0407606b4e42fe72d34add8037)",
    license: "MIT",
    source: "https://github.com/ggml-org/whisper.cpp",
    texts: licenseTexts(whisperDir),
  },
  {
    ecosystem: "model",
    name: "OpenAI Whisper model weights (ggml conversion: base, small; downloaded or imported by the user, not bundled)",
    version: "huggingface.co/ggerganov/whisper.cpp @ 5359861c739e955e79d9a303bcbc70fb988958b1",
    license: "MIT",
    source: "https://github.com/openai/whisper",
    texts: [],
  },
];

const all = [...crates, ...npm, ...native].sort(
  (a, b) => a.ecosystem.localeCompare(b.ecosystem) || a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
);

const violations = all.filter((c) => !allowed(c.license) && c.ecosystem !== "model");

// ---------- render ----------
const groups = new Map(); // license text hash -> { text, users[] }
for (const c of all) {
  for (const t of c.texts) {
    const h = createHash("sha256").update(t.text).digest("hex");
    if (!groups.has(h)) groups.set(h, { text: t.text, users: [] });
    groups.get(h).users.push(`${c.name} ${c.version} (${t.file})`);
  }
}
const lines = [
  "# Third-party notices",
  "",
  "<!-- Generated by scripts/notices.mjs; do not edit. `pnpm notices` regenerates, `pnpm notices:check` verifies. -->",
  "",
  "Local Dictation includes the following third-party software. Each component is listed with its declared",
  "license; the license texts found in the distributed sources follow, grouped where the text is identical.",
  "",
  "| Component | Version | License | Source |",
  "|---|---|---|---|",
  ...all.map((c) => `| ${c.name} | ${c.version} | ${c.license ?? "UNKNOWN"} | ${c.source} |`),
  "",
  "## License texts",
  "",
];
const sortedGroups = [...groups.values()].sort((a, b) => a.users[0].localeCompare(b.users[0]));
for (const g of sortedGroups) {
  lines.push(`### ${g.users.length === 1 ? g.users[0] : `${g.users[0]} and ${g.users.length - 1} more`}`, "");
  if (g.users.length > 1) lines.push(`Applies to: ${g.users.join(", ")}`, "");
  lines.push("```text", g.text.replace(/```/g, "'''"), "```", "");
}
const doc = lines.join("\n");

if (sbomAt) {
  const sbom = {
    bomFormat: "CycloneDX",
    specVersion: "1.5",
    version: 1,
    metadata: {
      component: {
        type: "application",
        name: "Local Dictation",
        version: JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version,
      },
    },
    components: all.map((c) => ({
      type: c.ecosystem === "model" ? "machine-learning-model" : "library",
      name: c.name,
      version: c.version,
      purl:
        c.ecosystem === "cargo"
          ? `pkg:cargo/${c.name}@${c.version}`
          : c.ecosystem === "npm"
            ? `pkg:npm/${c.name.replace("@", "%40")}@${c.version}`
            : undefined,
      licenses: c.license ? [{ expression: c.license }] : [],
      externalReferences: [{ type: "website", url: c.source }],
    })),
  };
  writeFileSync(sbomAt, `${JSON.stringify(sbom, null, 2)}\n`);
}

console.log(`${all.length} components (${crates.length} crates, ${npm.length} npm, ${native.length} native/model); ${groups.size} distinct license texts`);
if (violations.length) {
  console.error("License policy violations (not in the allowed list):");
  for (const v of violations) console.error(`  ${v.ecosystem} ${v.name} ${v.version}: ${v.license ?? "no license"}`);
}
if (check) {
  const current = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
  if (current !== doc) {
    console.error("THIRD_PARTY_NOTICES.md is stale: run `pnpm notices` and commit the result.");
    process.exit(1);
  }
  if (violations.length) process.exit(1);
  console.log("notices up to date; license policy ok");
} else {
  writeFileSync(OUT, doc);
  console.log(`wrote ${OUT}`);
  if (violations.length) process.exit(1);
}
