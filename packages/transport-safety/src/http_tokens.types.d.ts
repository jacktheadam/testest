/** Whether `code` is an RFC 7230 `tchar`. */
export function isTchar(code: number): boolean;
/** Whether `input[start..end)` is a non-empty run of `tchar`. */
export function isValidHttpTokenPart(input: unknown, start: number, end: number): boolean;
/** Whether `token` is a non-empty RFC 7230 token. */
export function isValidHttpToken(token: unknown): boolean;
