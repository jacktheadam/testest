// RFC 7230 defines a token as one or more `tchar`, and header field names and
// WebSocket subprotocol names are both tokens. Validating that a value really is
// one is what stops a header from being split or smuggled by a value that merely
// looks like a name.
//
// The character checks take a code point rather than a string so callers can
// validate a slice of a larger buffer without allocating a substring first.

function isTchar(code) {
  if (code >= 0x30 && code <= 0x39) return true; // 0-9
  if (code >= 0x41 && code <= 0x5a) return true; // A-Z
  if (code >= 0x61 && code <= 0x7a) return true; // a-z
  return (
    code === 0x21 || // !
    code === 0x23 || // #
    code === 0x24 || // $
    code === 0x25 || // %
    code === 0x26 || // &
    code === 0x27 || // '
    code === 0x2a || // *
    code === 0x2b || // +
    code === 0x2d || // -
    code === 0x2e || // .
    code === 0x5e || // ^
    code === 0x5f || // _
    code === 0x60 || // `
    code === 0x7c || // |
    code === 0x7e // ~
  );
}

// The type check is deliberate rather than redundant with the declaration: these
// run at trust boundaries where the input came off a socket, and a caller in
// plain JavaScript gets no compiler to hold it to the signature.
function isValidHttpTokenPart(input, start, end) {
  if (typeof input !== "string") return false;
  if (end <= start) return false;
  for (let i = start; i < end; i += 1) {
    if (!isTchar(input.charCodeAt(i))) return false;
  }
  return true;
}

function isValidHttpToken(token) {
  if (typeof token !== "string" || token.length === 0) return false;
  return isValidHttpTokenPart(token, 0, token.length);
}

export { isTchar, isValidHttpTokenPart, isValidHttpToken };
