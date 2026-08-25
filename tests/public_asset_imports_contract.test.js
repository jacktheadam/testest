/**
 * Assets under `apps/web/public/` are served verbatim at their own URL, so their
 * import specifiers resolve against the *URL*, not the filesystem. A relative path
 * that climbs out of the public root leaves whatever server is hosting the page and
 * 404s — silently, because a module worker that fails to load surfaces as an opaque
 * error event rather than a message naming the file.
 *
 * Two of these appeared at once when the browser tree moved and a path-rewriting
 * pass "corrected" them to point at `packages/`, which is right on disk and wrong
 * over HTTP. This makes the rule checkable instead of remembered.
 */
import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const publicRoot = path.join(repoRoot, "apps/web/public");

/** Every `from "..."` / `import("...")` specifier in a source file. */
function specifiers(source) {
  const out = [];
  const re = /(?:\bfrom|\bimport)\s*\(?\s*["']([^"']+)["']/g;
  for (;;) {
    const m = re.exec(source);
    if (!m) break;
    out.push(m[1]);
  }
  return out;
}

async function walk(dir) {
  const out = [];
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    const abs = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await walk(abs)));
    else if (/\.(js|mjs)$/.test(entry.name)) out.push(abs);
  }
  return out;
}

test("public assets never import from outside the public root", async () => {
  const files = await walk(publicRoot);
  assert.ok(files.length > 0, "expected scripts under apps/web/public");

  const offenders = [];
  for (const abs of files) {
    const source = await fs.readFile(abs, "utf8");
    for (const spec of specifiers(source)) {
      if (!spec.startsWith(".")) continue;
      const resolved = path.resolve(path.dirname(abs), spec);
      if (!resolved.startsWith(publicRoot + path.sep)) {
        offenders.push(`${path.relative(repoRoot, abs)} imports ${spec}`);
      }
    }
  }

  assert.deepEqual(
    offenders,
    [],
    "These resolve outside apps/web/public and will 404 when served. Use the generated copy " +
      `under public/_shared/, or add one:\n  ${offenders.join("\n  ")}`,
  );
});
