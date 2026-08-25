/**
 * A declaration that promises a function the runtime does not export is worse
 * than no declaration: it type-checks at every call site and fails only when the
 * code runs. These helpers are hand-written JavaScript with hand-written
 * declarations, so nothing else enforces the correspondence.
 *
 * Two shapes are covered:
 *
 * - **Dual-surface helpers** in `packages/transport-safety/`, consumed as
 *   CommonJS by the Node services and as ES modules by the browser host. One
 *   shared declaration describes both, and both runtime formats must satisfy it.
 * - **ES-module-only helpers** that live with the host in `src/`, where a
 *   declaration and its module sit side by side.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import { createRequire } from "node:module";

import { listFilesRecursive } from "./_helpers/fs_walk.js";

const require = createRequire(import.meta.url);

function declaredFunctionNames(dtsSource) {
  const names = [];
  const re = /^export function\s+([A-Za-z0-9_]+)\s*\(/gm;
  for (;;) {
    const match = re.exec(dtsSource);
    if (!match) break;
    names.push(match[1]);
  }
  return names;
}

function repoRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
}

test("transport-safety: both module surfaces export everything the declaration promises", async () => {
  const pkgRel = "packages/transport-safety/src";
  const pkgDir = path.resolve(repoRoot(), pkgRel);
  const files = new Set(await listFilesRecursive(pkgDir));

  const shared = [...files].filter((rel) => rel.endsWith(".types.d.ts")).sort();
  assert.ok(shared.length > 0, `Expected at least one shared declaration in ${pkgRel}`);

  for (const sharedRel of shared) {
    const base = sharedRel.replace(/\.types\.d\.ts$/, "");
    const esmRel = `${base}.js`;
    const cjsRel = `${base}.cjs`;

    assert.ok(files.has(esmRel), `Missing ES module surface ${pkgRel}/${esmRel}`);
    assert.ok(files.has(cjsRel), `Missing CommonJS implementation ${pkgRel}/${cjsRel}`);

    const declared = declaredFunctionNames(await readFile(path.resolve(pkgDir, sharedRel), "utf8"));
    assert.ok(declared.length > 0, `${pkgRel}/${sharedRel} declares no functions`);

    const esmMod = await import(pathToFileURL(path.resolve(pkgDir, esmRel)).href);
    const cjsMod = require(path.resolve(pkgDir, cjsRel));

    for (const fn of declared) {
      assert.equal(
        typeof esmMod[fn],
        "function",
        `${pkgRel}/${esmRel} must export ${fn}, declared in ${sharedRel}`,
      );
      assert.equal(
        typeof cjsMod[fn],
        "function",
        `${pkgRel}/${cjsRel} must export ${fn}, declared in ${sharedRel}`,
      );
    }
  }
});

test("host helpers: ES module exports match their declarations", async () => {
  const srcDir = path.resolve(repoRoot(), "apps/web/src");
  const files = await listFilesRecursive(srcDir);
  const fileSet = new Set(files);

  const stubs = files.filter((rel) => rel.endsWith(".d.ts") && !rel.endsWith(".cjs.d.ts")).sort();

  for (const stubRel of stubs) {
    const base = stubRel.replace(/\.d\.ts$/, "");
    const moduleRel = `${base}.js`;
    // Ambient declarations (global types, environment shims) have no module.
    if (!fileSet.has(moduleRel)) continue;

    const declared = declaredFunctionNames(await readFile(path.resolve(srcDir, stubRel), "utf8"));
    if (declared.length === 0) continue;

    const mod = await import(pathToFileURL(path.resolve(srcDir, moduleRel)).href);
    for (const fn of declared) {
      assert.equal(
        typeof mod[fn],
        "function",
        `src/${moduleRel} must export ${fn}, declared in src/${stubRel}`,
      );
    }
  }
});
