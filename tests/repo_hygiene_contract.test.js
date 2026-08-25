import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Repo-hygiene contract: banned paths/patterns must not be reintroduced.
//
// Context: the cleanup doctrine (see the wiki's cleanup plan) purged CI
// automation, project-theater files, duplicate/legacy stacks, and license
// files, and *banned* them from returning. The shell-side banned-paths check
// lives in `scripts/ci/check-repo-layout.sh`; this test mirrors it in the
// repo's JS contract-test surface so the ban is enforced by both gates.

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const BANNED_PATHS = [
  // CI/CD automation
  ".github",
  "codecov.yml",
  // Project theater (governance/legal simulation)
  "CODE_OF_CONDUCT.md",
  "CONTRIBUTING.md",
  "SECURITY.md",
  "LEGAL.md",
  "TRADEMARKS.md",
  "DMCA_POLICY.md",
  "TERMS_OF_SERVICE_TEMPLATE.md",
  "PRIVACY_POLICY_TEMPLATE.md",
  "AUTHORS",
  "NOTICE",
  "LICENSE-MIT",
  "LICENSE-APACHE",
  // Production-ization for imaginary deployments
  "deploy",
  "infra",
  "netlify.toml",
  "vercel.json",
  "docker-compose.yml",
  "compose.yaml",
  ".dockerignore",
  ".devcontainer",
  ".markdownlint-cli2.jsonc",
  // Legacy / duplicate stacks
  "server",
  "poc",
  "prototype",
  "guest",
  "instructions",
  "windows-drivers",
  "js",
  "windows",
  "images",
  "test-images",
  "tools/aero-gateway-rs",
  "REFACTOR.md",
];

// Glob-ish banned file names anywhere in the tree (license purge).
const BANNED_BASENAMES = new Set([
  "CODE_OF_CONDUCT.md",
  "LICENSE-MIT",
  "LICENSE-APACHE",
  "LICENSE.md",
  "netlify.toml",
  "vercel.json",
  "codecov.yml",
]);

function trackedFiles() {
  return execFileSync("git", ["ls-files"], {
    cwd: repoRoot,
    encoding: "utf8",
  })
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    // Skip files deleted in the working tree but not yet committed (purges
    // are uncommitted by policy, so git ls-files still lists them).
    .filter((line) => fs.existsSync(path.join(repoRoot, line)));
}

test("hygiene: banned paths must not exist", () => {
  const present = BANNED_PATHS.filter((p) => fs.existsSync(path.join(repoRoot, p)));
  assert.deepEqual(
    present,
    [],
    `banned path(s) reintroduced: ${present.join(", ")} — see the cleanup doctrine in the wiki`,
  );
});

test("hygiene: banned file names must not appear anywhere tracked", () => {
  const offenders = [];
  for (const rel of trackedFiles()) {
    const base = path.basename(rel);
    if (BANNED_BASENAMES.has(base)) offenders.push(rel);
  }
  assert.deepEqual(
    offenders,
    [],
    `banned file name(s) tracked: ${offenders.join(", ")} — see the cleanup doctrine in the wiki`,
  );
});

test("hygiene: no GitHub workflow/action configs anywhere tracked", () => {
  const offenders = trackedFiles().filter(
    (rel) => rel.startsWith(".github/") || /\.github\//.test(rel),
  );
  assert.deepEqual(offenders, [], `GitHub CI config reintroduced: ${offenders.join(", ")}`);
});

test("hygiene: no license-family files at repo root", () => {
  const rootFiles = fs
    .readdirSync(repoRoot, { withFileTypes: true })
    .filter((ent) => ent.isFile())
    .map((ent) => ent.name);
  const offenders = rootFiles.filter((name) =>
    /^(license|licence|copying|notice|authors|code_of_conduct|contributing|security|legal|trademarks?|dmca|terms_of_service|privacy_policy)/i.test(
      name,
    ),
  );
  assert.deepEqual(offenders, [], `license/theater file(s) at repo root: ${offenders.join(", ")}`);
});
