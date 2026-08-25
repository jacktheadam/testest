// Generate the browser-served copy of the one-line text helpers.
//
// Files under `web/public/` are served verbatim at their own URL — no bundler
// rewrites their imports — so a static asset there cannot reach into
// `packages/`. That is a real constraint, and the usual answer to it is a second
// hand-maintained copy plus a test comparing the two, which detects drift only
// after someone has written it.
//
// Generating the copy makes drift unrepresentable instead. The single
// implementation stays authoritative; this script mechanically converts it to an
// ES module, and `tests/public_text_asset_contract.test.js` fails if the
// committed asset stops matching what this produces.
//
// Run: node scripts/generate-public-text-one-line.mjs
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const SOURCE = path.join(root, "packages/transport-safety/src/text.js");
export const TARGET = path.join(root, "apps/web/public/_shared/text_one_line.js");

/** Re-emit the implementation as a standalone ES module for the public directory. */
export async function render() {
  const source = await readFile(SOURCE, "utf8");

  const exportsMatch = source.match(/export\s*\{([^}]*)\};?\s*$/);
  if (!exportsMatch) {
    throw new Error(`${SOURCE}: expected a trailing \`export { ... }\``);
  }
  const names = exportsMatch[1]
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  if (names.length === 0) {
    throw new Error(`${SOURCE}: no exported names found`);
  }

  const body = source.slice(0, exportsMatch.index).trimEnd();
  const banner = [
    "// GENERATED FILE — do not edit.",
    "//",
    "// Produced from packages/transport-safety/src/text.js by",
    "// scripts/generate-public-text-one-line.mjs, because assets under web/public/",
    "// are served verbatim and cannot import from packages/. Edit the source there",
    "// and re-run the script; tests/public_text_asset_contract.test.js checks this",
    "// file still matches.",
    "",
  ].join("\n");

  return `${banner}\n${body}\n\nexport { ${names.join(", ")} };\n`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await writeFile(TARGET, await render(), "utf8");
  console.error(`wrote ${path.relative(root, TARGET)}`);
}
