/**
 * Transport-safety declarations: one source of truth, two module surfaces.
 *
 * These helpers are consumed as CommonJS by the Node services and as ES modules
 * by the browser host, so each needs two entry points. They used to have two
 * hand-maintained *implementations* and two hand-maintained *declarations*, kept
 * in step by a test that compared them for equality — which catches drift only
 * after someone has already written it.
 *
 * There is now one implementation (`<name>.cjs`), one declaration
 * (`<name>.types.d.ts`), and thin surfaces that point at them. Drift is not
 * detected; it is unrepresentable. This test guards that structure, because the
 * structure is what makes the old parity check unnecessary.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { listFilesRecursive } from "./_helpers/fs_walk.js";

const PKG_REL = "packages/transport-safety/src";

function repoRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
}

async function implementationNames(pkgDir) {
  const files = await listFilesRecursive(pkgDir);
  return files
    .filter((rel) => rel.endsWith(".js") && !rel.endsWith(".d.ts") && !rel.endsWith(".types.js"))
    .map((rel) => path.posix.basename(rel, ".js"))
    .sort();
}

test("transport-safety: every helper has one implementation and one declaration", async () => {
  const pkgDir = path.resolve(repoRoot(), PKG_REL);
  const files = new Set(await listFilesRecursive(pkgDir));
  const names = await implementationNames(pkgDir);

  assert.ok(names.length > 0, `Expected at least one implementation under ${PKG_REL}`);

  for (const name of names) {
    // The implementation is the ES module; the CommonJS surface wraps it in one `require()`.
    //
    // It used to be the other way round, which was fine under Node and broken in the browser:
    // Vite serves a `.cjs` file inside the project verbatim, so an `export * from "./x.cjs"`
    // linked against a module with no ES exports and every import of it failed.
    assert.ok(files.has(`${name}.js`), `Missing ES module implementation ${PKG_REL}/${name}.js`);
    const cjs = await readFile(path.resolve(pkgDir, `${name}.cjs`), "utf8");
    assert.ok(
      cjs.includes(`require("./${name}.js")`),
      `${PKG_REL}/${name}.cjs must wrap the ES module, not carry its own copy`,
    );

    // Both declaration surfaces point at the same shared declaration.
    assert.ok(files.has(`${name}.types.d.ts`), `Missing shared declaration ${PKG_REL}/${name}.types.d.ts`);
    for (const surface of [`${name}.d.ts`, `${name}.cjs.d.ts`]) {
      assert.ok(files.has(surface), `Missing declaration surface ${PKG_REL}/${surface}`);
      const text = await readFile(path.resolve(pkgDir, surface), "utf8");
      assert.ok(
        text.includes(`export * from "./${name}.types.js";`),
        `${PKG_REL}/${surface} must point at the shared declaration, not restate it`,
      );
    }
  }
});

test("transport-safety: the helpers are not copied back into the host or services", async () => {
  // The point of the package is that these exist once. A copy reappearing
  // elsewhere is the failure mode this replaces the old parity check for.
  const root = repoRoot();
  const names = await implementationNames(path.resolve(root, PKG_REL));

  const offenders = [];
  for (const searchRoot of ["src", "services", "tools", "apps/web"]) {
    let rels;
    try {
      rels = await listFilesRecursive(path.resolve(root, searchRoot));
    } catch {
      continue;
    }
    for (const rel of rels) {
      if (rel.includes("node_modules") || rel.includes("dist/")) continue;
      const base = path.posix.basename(rel).replace(/\.(cjs\.d\.ts|d\.ts|cjs|js)$/, "");
      if (names.includes(base)) offenders.push(`${searchRoot}/${rel}`);
    }
  }

  assert.deepEqual(
    offenders,
    [],
    `These helpers live only in ${PKG_REL}; found copies:\n  ${offenders.join("\n  ")}`,
  );
});
