/** Collapse whitespace runs to single spaces and drop control characters. */
export function sanitizeOneLine(input: unknown): string;
/** Truncate to at most `maxBytes` UTF-8 bytes, never splitting a character. */
export function truncateUtf8(input: unknown, maxBytes: number): string;
/** {@link sanitizeOneLine} and {@link truncateUtf8} in one pass. */
export function formatOneLineUtf8(input: unknown, maxBytes: number): string;

/** How `err.name` may stand in when no usable message is present. */
export type NameFallbackMode = "never" | "missing" | "always";

export interface FormatOneLineErrorOptions {
  /** `true` maps to "always", `"missing"` to "missing", anything else to "never". */
  includeNameFallback?: boolean | "missing";
}

/** Read a message off an arbitrary thrown value safely, then format it. */
export function formatOneLineError(
  err: unknown,
  maxBytes: number,
  fallbackOrOptions?: string | FormatOneLineErrorOptions,
): string;

/** The message-extraction step on its own. */
export function safeErrorMessageInput(err: unknown, nameFallbackMode?: NameFallbackMode): string;
