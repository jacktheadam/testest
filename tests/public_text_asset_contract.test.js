/**
 * The one-line text helpers exist once, and the browser copy is generated.
 *
 * `sanitizeOneLine`, `truncateUtf8`, `formatOneLineUtf8`, and `formatOneLineError`
 * used to exist as seven hand-maintained copies across the host, two services,
 * two tools, the bench dashboard, and the browser-served assets — kept in step by
 * tests that compared them pairwise. Comparison catches drift only after someone
 * has already written it, and two of those copies had in fact drifted: they
 * disagreed on what `formatOneLineError`'s third argument even meant.
 *
 * There is now one implementation, in `packages/transport-safety`. Everything
 * that runs under Node imports it. The one exception is `web/public/_shared/`,
 * which is served verbatim at its own URL and so cannot import from `packages/`;
 * that file is generated from the implementation rather than written.
 *
 * This test guards both halves: the generated asset matches its source, and the
 * merged `formatOneLineError` still honours the two call shapes it reconciled.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";

import { render, TARGET } from "../scripts/generate-public-text-one-line.mjs";

const require = createRequire(import.meta.url);
const text = require("../packages/transport-safety/src/text.cjs");

test("the browser asset matches what the generator produces", async () => {
  const committed = await readFile(TARGET, "utf8");
  assert.equal(
    committed,
    await render(),
    "apps/web/public/_shared/text_one_line.js is stale - run `node scripts/generate-public-text-one-line.mjs`",
  );
});

test("sanitizeOneLine collapses whitespace and drops control characters", () => {
  const cases = [
    ["", ""],
    ["  a  ", "a"],
    ["a\tb\nc", "a b c"],
    ["a\u0000b", "a b"], // NUL
    ["\u0000", ""], // NUL on its own
    ["a\u2028b", "a b"], // LINE SEPARATOR
    ["a\u2029b", "a b"], // PARAGRAPH SEPARATOR
    ["a\u00a0b", "a b"], // NBSP
  ];
  for (const [input, expected] of cases) {
    assert.equal(text.sanitizeOneLine(input), expected, `sanitizeOneLine(${JSON.stringify(input)})`);
  }

  // A hostile value must not be able to throw out of a logging call.
  const throwing = {
    toString() {
      throw new Error("boom");
    },
  };
  assert.equal(text.sanitizeOneLine(throwing), "");
});

test("truncateUtf8 counts encoded bytes and never splits a character", () => {
  assert.equal(text.truncateUtf8("hello", 5), "hello");
  assert.equal(text.truncateUtf8("hello", 4), "hell");
  assert.equal(text.truncateUtf8("€", 3), "€", "a 3-byte character fits in exactly 3 bytes");
  assert.equal(text.truncateUtf8("€", 2), "", "and is dropped whole rather than halved");
  assert.equal(text.truncateUtf8("🙂", 4), "🙂");
  assert.equal(text.truncateUtf8("🙂", 3), "");
  assert.equal(text.truncateUtf8("abc", 0), "");
  assert.equal(text.truncateUtf8("abc", -1), "");
});

test("formatOneLineError accepts both of the call shapes it was merged from", () => {
  // The shape the host and services used: a fallback string for when nothing
  // usable could be extracted.
  assert.equal(text.formatOneLineError(new Error("x\ny"), 512), "x y");
  assert.equal(text.formatOneLineError({ message: "\n" }, 512, "custom"), "custom");
  assert.equal(text.formatOneLineError(null, 512), "null");

  // The shape the browser assets used: a name-fallback mode.
  assert.equal(text.formatOneLineError({ name: "Boom" }, 512, { includeNameFallback: false }), "Error");
  assert.equal(text.formatOneLineError({ name: "Boom" }, 512, { includeNameFallback: "missing" }), "Boom");
  assert.equal(
    text.formatOneLineError({ message: "", name: "Boom" }, 512, { includeNameFallback: "missing" }),
    "Error",
    '"missing" defers to an empty message rather than overriding it',
  );
  assert.equal(
    text.formatOneLineError({ message: "", name: "Boom" }, 512, { includeNameFallback: true }),
    "Boom",
    '"always" steps in when the message is empty',
  );
});

test("formatOneLineError survives a value that fights back", () => {
  const throwingMessage = {
    get message() {
      throw new Error("boom");
    },
  };
  assert.equal(text.formatOneLineError(throwingMessage, 512), "Error");

  const throwingName = {
    get name() {
      throw new Error("boom");
    },
  };
  assert.equal(text.formatOneLineError(throwingName, 512, { includeNameFallback: true }), "Error");
});
