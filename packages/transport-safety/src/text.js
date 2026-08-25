// One-line, byte-bounded text formatting for anything that reaches a log, a
// WebSocket close reason, or an HTTP status line.
//
// Two hazards drive the shape of this. Untrusted text containing newlines or the
// Unicode line separators forges log entries and splits headers, so every
// whitespace run collapses to a single space and control characters are dropped
// outright. And the byte budgets these values face — a close reason is 123 bytes —
// are counted in UTF-8, not code units, so truncation has to happen at an encoded
// boundary or it produces a replacement character at the end.
//
// `encodeInto` gives both at once: it fills a fixed buffer and reports how far it
// got, so a multi-byte character that does not fit is left out entirely rather
// than cut in half.

const UTF8 = Object.freeze({ encoding: "utf-8" });
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder(UTF8.encoding);

function coerceString(input) {
  try {
    return String(input ?? "");
  } catch {
    return "";
  }
}

function sanitizeOneLine(input) {
  const parts = [];
  let hasOutput = false;
  let pendingSpace = false;
  for (const ch of coerceString(input)) {
    const code = ch.codePointAt(0) ?? 0;
    const forbidden = code <= 0x1f || code === 0x7f || code === 0x85 || code === 0x2028 || code === 0x2029;
    if (forbidden || /\s/u.test(ch)) {
      pendingSpace = hasOutput;
      continue;
    }
    if (pendingSpace) {
      parts.push(" ");
      pendingSpace = false;
    }
    parts.push(ch);
    hasOutput = true;
  }
  return parts.join("");
}

function truncateUtf8(input, maxBytes) {
  if (!Number.isInteger(maxBytes) || maxBytes < 0) return "";
  const s = coerceString(input);
  if (maxBytes === 0) return "";
  const buf = new Uint8Array(maxBytes);
  const { read, written } = textEncoder.encodeInto(s, buf);
  if (read === s.length) return s;
  return written === 0 ? "" : textDecoder.decode(buf.subarray(0, written));
}

function formatOneLineUtf8(input, maxBytes) {
  if (!Number.isInteger(maxBytes) || maxBytes < 0) return "";
  if (maxBytes === 0) return "";

  const buf = new Uint8Array(maxBytes);
  let written = 0;
  let pendingSpace = false;
  for (const ch of coerceString(input)) {
    const code = ch.codePointAt(0) ?? 0;
    const forbidden = code <= 0x1f || code === 0x7f || code === 0x85 || code === 0x2028 || code === 0x2029;
    if (forbidden || /\s/u.test(ch)) {
      pendingSpace = written > 0;
      continue;
    }

    if (pendingSpace) {
      const spaceRes = textEncoder.encodeInto(" ", buf.subarray(written));
      if (spaceRes.written === 0) break;
      written += spaceRes.written;
      pendingSpace = false;
      if (written >= maxBytes) break;
    }

    const res = textEncoder.encodeInto(ch, buf.subarray(written));
    if (res.written === 0) break;
    written += res.written;
    if (written >= maxBytes) break;
  }
  return written === 0 ? "" : textDecoder.decode(buf.subarray(0, written));
}

// Pulling a message off an arbitrary thrown value without letting it misbehave.
//
// A `catch` binding can hold anything, including an object whose `message` getter
// throws or whose `toString` is expensive or hostile, so this reads only what it
// can read safely and never calls `toString` on an object or function.
//
// `nameFallbackMode` decides what happens when there is no usable message:
//
//   "never"    — give up and let the caller's fallback apply. The default,
//                because an error class name rarely tells a reader anything.
//   "missing"  — use `err.name` only when there is no `message` property at all.
//   "always"   — use `err.name` when the message is absent *or* empty, which is
//                what a `DOMException`-style rejection with a blank message needs
//                to say anything at all.
function safeErrorMessageInput(err, nameFallbackMode) {
  if (err === null) return "null";

  const t = typeof err;
  if (t === "string") return err;
  if (t === "number" || t === "boolean" || t === "bigint" || t === "symbol" || t === "undefined") {
    return String(err);
  }
  if (t !== "object") return "Error";

  const allowNameFallback = nameFallbackMode === "always" || nameFallbackMode === "missing";
  const allowNameWhenMessageEmpty = nameFallbackMode === "always";

  try {
    const hasMessage = err && typeof err.message === "string";
    const msg = hasMessage ? err.message : "";
    if (hasMessage && !allowNameWhenMessageEmpty) return msg;
    if (msg) return msg;
  } catch {
    // A throwing `message` getter is exactly the case this guards.
  }

  if (allowNameFallback) {
    try {
      const name = err && typeof err.name === "string" ? err.name : "";
      if (name) return name;
    } catch {
      // As above.
    }
  }

  return "Error";
}

// The third parameter carries two different intentions, distinguished by type
// because both spellings are in use and neither is wrong:
//
//   a string — the text to show when nothing usable could be extracted
//   an object — `{ includeNameFallback }`, selecting the mode described above
function formatOneLineError(err, maxBytes, fallbackOrOptions) {
  let fallback = "Error";
  let nameFallbackMode = "never";

  if (typeof fallbackOrOptions === "string") {
    if (fallbackOrOptions) fallback = fallbackOrOptions;
  } else if (fallbackOrOptions && typeof fallbackOrOptions === "object") {
    const requested = fallbackOrOptions.includeNameFallback;
    nameFallbackMode = requested === "missing" ? "missing" : requested ? "always" : "never";
  }

  const raw = safeErrorMessageInput(err, nameFallbackMode);
  return formatOneLineUtf8(raw, maxBytes) || fallback;
}

export { sanitizeOneLine, truncateUtf8, formatOneLineUtf8, formatOneLineError, safeErrorMessageInput };
