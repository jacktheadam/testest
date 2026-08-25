/**
 * Repo hygiene: the cleanup ban list, enforced.
 *
 * The cleanup doctrine (see the ban list in the wiki working agreements) purges project theater — CI/CD
 * automation, bot dependency bumps, governance and legal templates,
 * production-ization configs for deployments that do not exist — and forbids it
 * coming back. It also forbids reintroducing a duplicate stack once one has been
 * retired, and forbids a second documentation tree outside the wiki.
 *
 * A ban that lives only in a document is a convention, and conventions decay:
 * each individual reintroduction looks locally reasonable, and nobody re-reads
 * the doctrine first. This test makes the ban a gate.
 *
 * Adding an exception is deliberate: change this file, and record the reasoning
 * in the cleanup plan's decision log in the same change.
 */
import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Paths that must not exist, with the reason each was banned. */
const BANNED_PATHS = [
  // CI/CD automation. Verification here is run by an agent, not by a platform.
  [".github", "CI/CD automation is banned; gates are commands anyone can run"],
  ["codecov.yml", "coverage-service configuration is CI theater"],
  [".gitlab-ci.yml", "CI/CD automation is banned"],
  ["azure-pipelines.yml", "CI/CD automation is banned"],
  ["Jenkinsfile", "CI/CD automation is banned"],

  // Project theater: governance and legal writing for a public project Aero is not.
  ["CODE_OF_CONDUCT.md", "project theater: no contributor community to govern"],
  ["CONTRIBUTING.md", "project theater: no external contribution queue"],
  ["SECURITY.md", "project theater: no security disclosure process"],
  ["GOVERNANCE.md", "project theater"],
  ["AUTHORS", "project theater: roll call"],
  ["NOTICE", "project theater: roll call"],
  ["DMCA_POLICY.md", "project theater: legal template"],
  ["TRADEMARKS.md", "project theater: legal template"],
  ["LEGAL.md", "project theater: legal template"],
  ["TERMS_OF_SERVICE_TEMPLATE.md", "project theater: legal template"],
  ["PRIVACY_POLICY_TEMPLATE.md", "project theater: legal template"],
  ["LICENSE", "the license-files decision purged all of them"],
  ["LICENSE-MIT", "the license-files decision purged all of them"],
  ["LICENSE-APACHE", "the license-files decision purged all of them"],

  // Production-ization for deployments that do not exist.
  ["deploy", "deployment production-ization is out of scope"],
  ["infra", "infrastructure-as-code for imaginary deployments is out of scope"],
  [".devcontainer", "hosted-development theater"],
  ["docker-compose.yml", "multi-host deployment stack is out of scope"],
  ["compose.yaml", "multi-host deployment stack is out of scope"],
  ["netlify.toml", "hosting-provider configuration is out of scope"],
  ["vercel.json", "hosting-provider configuration is out of scope"],

  // Duplicate stacks, retired and not to be reintroduced.
  ["server", "retired; the gateway lives at services/gateway"],
  ["tools/aero-gateway-rs", "retired; superseded by services/gateway and aero-l2-proxy"],
  ["services/image-gateway", "retired; a production reference implementation, not a dependency"],
  ["docs", "the wiki is the only documentation home"],
  ["instructions", "sprint-era workstream briefs; absorbed into wiki/history"],
  ["poc", "retired prototype tree"],
  ["prototype", "retired prototype tree"],
  ["guest", "retired tombstone tree"],
  ["windows-drivers", "stale duplicate of drivers/windows7"],
  [
    "crates/emulator",
    "retired parallel device stack; the canonical stack is aero-machine plus the aero-* device crates",
  ],
  ["images", "placeholder directory"],
  ["test-images", "placeholder directory"],

  // One package manager. The npm-era artefacts are retired.
  ["package-lock.json", "pnpm is the package manager; pnpm-lock.yaml is the only lockfile"],
  ["yarn.lock", "pnpm is the package manager"],
];

test("repo hygiene: banned paths have not reappeared", async () => {
  const offenders = [];
  for (const [rel, reason] of BANNED_PATHS) {
    try {
      await fs.stat(path.join(repoRoot, rel));
      offenders.push(`${rel} — ${reason}`);
    } catch {
      // Absent, as required.
    }
  }
  assert.deepEqual(
    offenders,
    [],
    `Banned paths reappeared. Each is banned by the ban list in the wiki working agreements; if one is now justified, ` +
      `remove it from this test and record the decision there in the same change:\n  ` +
      offenders.join("\n  "),
  );
});

test("repo hygiene: the wiki is the only documentation tree", async () => {
  // A second docs tree is how documentation drifts: two homes means two truths,
  // and readers find whichever they hit first.
  const entries = await fs.readdir(repoRoot, { withFileTypes: true });
  const docLike = entries
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .filter((name) => /^(docs?|documentation|handbook)$/i.test(name) && name !== "wiki");

  assert.deepEqual(docLike, [], `Documentation must live in wiki/ only; found: ${docLike.join(", ")}`);
});

test("repo hygiene: exactly one JavaScript lockfile and one workspace declaration", async () => {
  const present = [];
  for (const f of ["pnpm-lock.yaml", "package-lock.json", "yarn.lock", "bun.lockb"]) {
    try {
      await fs.stat(path.join(repoRoot, f));
      present.push(f);
    } catch {
      // Absent.
    }
  }
  assert.deepEqual(present, ["pnpm-lock.yaml"], `Expected only pnpm-lock.yaml; found: ${present.join(", ")}`);

  // Two workspace declarations drift apart; pnpm-workspace.yaml is authoritative.
  const pkg = JSON.parse(await fs.readFile(path.join(repoRoot, "package.json"), "utf8"));
  assert.equal(
    pkg.workspaces,
    undefined,
    "package.json must not declare `workspaces`; pnpm-workspace.yaml is the single declaration",
  );
  await fs.stat(path.join(repoRoot, "pnpm-workspace.yaml"));
});

test("repo hygiene: package scripts use one package manager", async () => {
  // Mixing package managers is how a migration stalls half-done: `npm -w` stops
  // resolving the moment the npm workspace declaration goes away, and the
  // failure surfaces far from the change that caused it.
  const manifests = ["package.json"];
  const workspaceYaml = await fs.readFile(path.join(repoRoot, "pnpm-workspace.yaml"), "utf8");
  for (const m of workspaceYaml.matchAll(/^\s*-\s*"?([^"\n]+)"?\s*$/gm)) {
    const pattern = m[1].trim();
    if (pattern.endsWith("/*")) {
      const base = pattern.slice(0, -2);
      let names = [];
      try {
        names = await fs.readdir(path.join(repoRoot, base));
      } catch {
        continue;
      }
      for (const n of names) manifests.push(path.posix.join(base, n, "package.json"));
    } else {
      manifests.push(path.posix.join(pattern, "package.json"));
    }
  }

  const offenders = [];
  for (const rel of manifests) {
    let raw;
    try {
      raw = await fs.readFile(path.join(repoRoot, rel), "utf8");
    } catch {
      continue;
    }
    const scripts = JSON.parse(raw).scripts ?? {};
    for (const [name, body] of Object.entries(scripts)) {
      if (typeof body !== "string") continue;
      // `npx` is fine — it is not a package manager invocation.
      if (/\bnpm\s+(run|-w|--workspace|ci|install|exec|test)\b/.test(body)) {
        offenders.push(`${rel} :: ${name} :: ${body}`);
      }
    }
  }

  assert.deepEqual(offenders, [], `Scripts must invoke pnpm, not npm:\n  ${offenders.join("\n  ")}`);
});
