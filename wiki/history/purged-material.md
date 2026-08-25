# Purged material

> **Historical.** Documents removed from the tree during cleanup, preserved
> verbatim with the reason for removal.
>
> Two kinds appear here. The first is *project theater* — governance, legal,
> and policy templates for a public project Aero is not — purged because it
> misleads every future reader about what this repo is. The second is
> superseded or out-of-scope subtrees, whose useful content was absorbed
> elsewhere before removal. The graveyard index with the reasoning for each
> removal is [retirements.md](./retirements.md).

Your job is to thoroughly grok, explore, review, improve and refactor the codebase and system, systematically. This is a complex large task — you can do it fully, without hesitation, without narrowing. Work step by step.

Take full ownership and responsibility over entire codebase. Have full ambition, do not hold back, do not take shortcuts. You have full autonomy and agency to do whatever is in your judgement, taste. Use all your skills, world knowledge. You do not have to care about backwards compatibility, existing code, existing decisions, existing structure. You are free to do anything: big refactors, rip things out, majorly restructure.

Use scratchpad.md as your scratchpad — use it frequently, update before end of turn. Do not commit or push this scratchpad.md file.

This is a complex large-scale engineering project; use good judgement. Don't rush, take step back, do thoughtful elegant design and decisions. Make good engineering decisions: patterns, abstractions, modularity, separation of concerns, no unnecessary coupling, least surface area, testable, composable, self contained. Think of the great engineering patterns.

Simplicity and elegance and beauty is best: faster, easier to get correct, easier to understand and maintain; self contained and self describing. Not just code, but a mindset, worldview: packaging, CI, design, structure, etc. Be bold, modern, fresh, first principles (not just barebones for barebones sake, radical for radical sake, different for different sake, etc.)

Do the hard things. Work autonomously. Take responsibility over entire project — not just code, but docs, comments, organization, structure, etc. Make it fast to compile, easy to test. Avoid strange, weird, frictional patterns, devex. Record the meta: if you've decided on coding standards, design decisions, engineering patterns, etc., document these too for the project and future devs.

Do not do hacky code, fallbacks, shortcuts, workarounds, TODOs, partial implementations, sloppy code.

Make regular commits and push them frequently.

## Refactor plan (phased, test-backed)

### Phase 1: Bound attacker-controlled work (done)
Goal: ensure every network / protocol / cross-worker boundary is defensively parsed and cannot reflect or allocate on untrusted strings without conservative caps.

Outcomes:
- Client-visible error strings are single-line and UTF-8 byte-bounded by default.
- WebSocket upgrade parsing rejects oversized request targets/headers early and deterministically.
- Protocol error payloads are capped consistently (e.g. tcp-mux error message byte cap).
- Helpers that could become “future footguns” (sendText/respondError/etc.) clamp/sanitize defensively even when current call sites are constant.

### Phase 2: Prevent drift with parity/contract tests (done)
Goal: once invariants are in place, ensure they cannot silently drift across implementations.

Outcomes:
- Repo-root parity tests cover shared primitives that are imported by multiple runtimes (Node/TS/JS shims):
  - text sanitization + UTF-8 truncation
  - RFC7230 token validation
  - subprotocol parsing behavior
  - tcp-mux error message byte cap consistency
  - raw header scanning contract semantics

### Phase 3: Module boundaries and test strategy (done)
Goal: reduce “ESM/CJS impedance mismatch” and keep tests aligned with package runtime semantics.

Approach:
- Treat each workspace package as the authority for its own module format.
- Prefer package-local tests for package-internal TS sources when the package is CJS (avoid importing `src/*.ts` directly from repo-root ESM tests).
- Where cross-package parity is needed, compare either:
  - the ESM-exported shared utilities (repo-root `src/*`), or
  - the built artifacts for CJS packages (via their own test runners).

Outcomes:
- A repo-root module boundary contract test prevents “module format accident” imports across test trees.
- Workspace package module formats are explicit where needed (`"type": "module"` vs `"type": "commonjs"`).
- A fast repo-root contract runner (`npm run test:contracts`) exists for quick sanity checks.

### Phase 4: Opportunistic cleanup (done)
Goal: keep surfaces small, remove duplication, and simplify without weakening the invariants above.

Scope:
- Delete dead code and redundant wrappers.
- Tighten error typing and response construction helpers.
- Normalize validation boundaries so “unsafe defaults” cannot reappear.

Outcomes:
- Network/protocol surfaces use stable, single-line, UTF-8 byte-bounded client-visible errors by default.
- Shared parsing/formatting invariants are guarded by parity/contract tests to prevent silent drift.

### Phase 5: Cross-platform drift guards (done)
Goal: catch portability bugs (path separators, platform-specific tooling quirks) early and cheaply.

Approach:
- Keep the contract/parity suite fast enough to run on multiple OSes.
- Add targeted CI jobs when a portability bug is plausible and expensive jobs already exist.

Outcomes:
- A lightweight Windows CI job runs `npm run test:contracts` to exercise the contract/parity suite under Windows path semantics (see `node-contracts-windows` in `.github/workflows/ci.yml`).

### Phase 6: Contract-suite portability hardening (done)
Goal: keep the contract/parity suite correct under both POSIX and Windows path semantics, and make its intent harder to accidentally subvert.

Approach:
- Treat *filesystem paths* and *module specifiers* as distinct domains; normalize explicitly when converting between them.
- Where tests use regex scans (instead of AST parsing), ensure the patterns match both `/` and escaped Windows separators (`\\`) when appropriate.

Outcomes:
- Module-boundary scanning is robust to `..\\workspace\\src\\file.ts`-style specifiers (and resolves `.js` existence checks portably).

### Phase 7: CI action portability (done)
Goal: keep CI steps reliable across OSes by minimizing dependence on runner-specific shells.

Approach:
- Prefer Node scripts over bash for composite action logic (inputs, path resolution, output writing).
- Lock the behavior in with small contract tests so the contract suite exercises CI-critical parsing.

Outcomes:
- `setup-node-workspace` no longer requires bash for version/workspace detection, and its behavior is covered by the contract suite.

### Phase 8: CI action portability sweep (done)
Goal: reduce runner shell coupling across the remaining composite actions in `.github/actions/` (especially for Windows jobs).

Approach:
- Replace bash-only parsing logic with Node scripts (ESM) that explicitly handle paths and write `GITHUB_OUTPUT`/`GITHUB_ENV`.
- Keep scripts dependency-free and make command execution safe (`execFileSync` / `spawnSync` with `shell: false`).
- Add small contract tests for any new CI-parsing logic that would be painful to debug in CI.

Outcomes:
- `setup-rust`, `setup-playwright`, and `resolve-wasm-crate` no longer require bash for their internal logic.

### Phase 9: CI action script consolidation (done)
Goal: keep CI action scripts small and consistent by deduplicating common “GitHub IO” utilities (outputs/env/path normalization) without changing behavior.

Approach:
- Add a tiny shared helper module under `.github/actions/_shared/` for:
  - `GITHUB_OUTPUT` / `GITHUB_ENV` append helpers
  - error formatting + exit behavior
  - small path normalization helpers used across actions
- Refactor action-local scripts to import these helpers instead of reimplementing them.

Outcomes:
- CI action scripts share a single, well-tested implementation for output/env writing and path normalization.

### Phase 10: Composite action cwd robustness (done)
Goal: ensure composite actions work regardless of step working directory by avoiding repo-relative script paths.

Approach:
- Use `${{ github.action_path }}` when invoking action-local scripts so paths are cwd-independent.
- Add a contract test to prevent regressions (no `node .github/actions/...` in composite actions).

Outcomes:
- All composite actions invoke action-local scripts via `github.action_path`.

### Phase 11: Contract test helper consolidation (done)
Goal: keep the contract suite easy to maintain by deduplicating common test utilities.

Approach:
- Add a small `tests/_helpers` module for:
  - running Node scripts with env overrides (for action script contract tests)
  - parsing `GITHUB_OUTPUT`/`GITHUB_ENV` key/value files
- Refactor existing action contract tests to use the helpers (no behavior changes).

Outcomes:
- Action contract tests share one implementation for “run Node script” + “parse key/value output files”, reducing drift.

### Phase 12: CI action defensive execution (done)
Goal: avoid CI hangs and resource spikes by bounding “helper” command execution inside action scripts.

Approach:
- Add small `_shared` helpers for spawning subprocesses with timeouts / bounded buffers.
- Apply timeouts to detection/dry-run helpers where long runtimes indicate a hang (not real work).
- Add contract tests for parsing logic that depends on subprocess output (e.g. Playwright dry-run parsing).

Outcomes:
- Action scripts use timeouts for detection/dry-run subprocesses and have contract coverage for their parsing logic.

### Phase 13: Guardrail hardening (done)
Goal: keep the repo resilient to accidental regressions by expanding “cheap, high-signal” contracts.

Approach:
- Make CI guardrail tests discover targets automatically (e.g. scan all composite actions) so new additions inherit the guardrails by default.
- Keep rules conservative: flag only patterns that are known to be brittle (e.g. repo-relative action script paths).

Outcomes:
- The composite-action path contract automatically covers all actions under `.github/actions/`.

### Phase 14: Composite action shell guardrails (done)
Goal: prevent reintroducing brittle cross-OS patterns in composite actions (especially for Windows runners).

Approach:
- Add a contract test that scans all `.github/actions/**/action.yml` files and forbids:
  - `shell: bash` (forces a non-default shell on Windows)
  - heredoc-style inline scripts (`<<'NODE'`, etc.) that are shell-dependent

Outcomes:
- Composite actions remain shell-agnostic by default; regressions are caught by the contract suite.

### Phase 15: Guardrail test deduplication (done)
Goal: keep the guardrail contract tests concise by sharing common “action discovery” logic.

Approach:
- Add `tests/_helpers/github_actions_contract_helpers.js` to centralize discovery of `.github/actions/**/action.yml`.
- Refactor guardrail tests to use the helper.

Outcomes:
- Guardrail tests share a single implementation for composite action discovery.

### Phase 16: GitHub action output hardening (done)
Goal: reduce subtle CI breakage by centralizing correct `GITHUB_OUTPUT` multiline writing semantics.

Approach:
- Add shared helpers for multiline outputs (delimiter handling) in `.github/actions/_shared`.
- Update action scripts to use the shared helper instead of hand-rolled delimiter formatting.
- Lock in behavior with contract tests.

Outcomes:
- Action scripts use shared multiline output helpers, and contract tests validate delimiter formatting.

### Phase 17: GitHub action output/env correctness guards (done)
Goal: prevent accidental invalid writes to `GITHUB_OUTPUT` / `GITHUB_ENV` that can silently corrupt downstream steps.

Approach:
- Reject newline-containing values for single-line output/env helpers (`appendOutput`/`appendEnv`) and provide clear guidance to use multiline helpers.
- Add `appendMultilineEnv` to match `appendMultilineOutput`.
- Add contract coverage by spawning a node process to observe exit behavior (since helpers terminate on invalid inputs).

Outcomes:
- Shared helpers enforce correct GitHub file command formats and are covered by contract tests.

### Phase 18: Action subprocess helper consolidation (done)
Goal: keep composite action scripts consistent and defensive by centralizing subprocess execution patterns.

Approach:
- Extend `.github/actions/_shared/exec.mjs` with helpers for invoking Node-based CLIs with consistent encoding/timeouts.
- Refactor action scripts to use the shared helpers rather than ad-hoc `execFileSync` calls.

Outcomes:
- Action scripts share one implementation for “run Node CLI, capture UTF-8 output” and “run Node CLI with inherited stdio”.

### Phase 19: JS eval sink guardrails (done)
Goal: prevent accidental introduction of JavaScript eval sinks (`eval`, `new Function`) in production code paths.

Approach:
- Add a contract test that scans the repo’s JS/TS sources (excluding tests) for eval sinks.
- Keep the patterns conservative and focused on real sinks (direct `eval(` / `globalThis.eval(` / `new Function(`).

Outcomes:
- Contract suite fails if eval sinks appear in production code.

### Phase 20: DOM XSS sink guardrails (done)
Goal: prevent accidental introduction of unsafe DOM HTML injection patterns in production code.

Approach:
- Add a contract test that scans production JS/TS sources (excluding tests) for common XSS sinks:
  - `dangerouslySetInnerHTML`
  - `.innerHTML`, `.outerHTML`, `.insertAdjacentHTML`
- Mask strings/comments to avoid false positives from embedded text.

Outcomes:
- Contract suite fails if new DOM HTML injection sinks appear in production sources.

### Phase 21: JS source-scan consolidation + subprocess guardrails (done)
Goal: reduce duplicated source-scanner logic while adding a guardrail against unsafe subprocess APIs.

Approach:
- Add `tests/_helpers/js_source_scan_helpers.js` to centralize production JS/TS file discovery and string/comment masking.
- Refactor existing JS guardrail tests to use the helper.
- Add a new contract test that forbids `child_process.exec/execSync` imports and `shell: true` in production JS/TS sources.

Outcomes:
- JS guardrail tests share a single scanner implementation, and the contract suite blocks unsafe subprocess APIs in production sources.

### Phase 22: SQL injection guardrails (done)
Goal: prevent accidental introduction of known-unsafe Prisma raw query APIs.

Approach:
- Add a contract test that scans production JS/TS sources for Prisma unsafe raw query methods:
  - `queryRawUnsafe`, `executeRawUnsafe`, `$queryRawUnsafe`, `$executeRawUnsafe`
- Mask strings/comments to avoid false positives from documentation text.

Outcomes:
- Contract suite fails if Prisma unsafe raw query APIs appear in production sources.

### Phase 23: Expand DOM XSS sink guardrails (done)
Goal: broaden DOM sink guardrails to cover additional classic HTML injection APIs.

Approach:
- Extend the DOM XSS contract to forbid:
  - `document.write` / `document.writeln`
  - `Range.createContextualFragment`
- Keep masking of strings/comments to avoid false positives.

Outcomes:
- Contract suite blocks these additional DOM HTML injection sinks in production sources.

### Phase 24: JS source-scan correctness hardening (done)
Goal: make the JS/TS source masking helper more faithful to real JS syntax so security guardrails don’t miss code due to lexer edge cases.

Approach:
- Harden `stripStringsAndComments` to correctly handle:
  - Regex literals (so `/\//` doesn’t get misread as a `//` comment)
  - Nested strings/comments/regex inside template expressions (`\`${ ... }\``) without losing template-expression context
- Add focused contract coverage for these edge cases.

Outcomes:
- `tests/js_source_scan_helpers_contract.test.js` locks in masking behavior for regex + template-expression cases.

### Phase 25: Subprocess sink guardrail correctness (done)
Goal: ensure the subprocess sink contract actually detects forbidden `child_process` exec/execSync patterns without being defeated by string masking.

Approach:
- Refactor the subprocess sink scan to:
  - Use masked code only to find candidate `import`/`require` tokens (avoids matches in strings/comments/regex).
  - Parse the module specifier from the original source to correctly detect `"child_process"` / `"node:child_process"`.
- Add focused parsing contract coverage for the sink scanner.

Outcomes:
- `tests/js_subprocess_sinks_parsing_contract.test.js` ensures `import { exec } from "child_process"` and `require("child_process").execSync(` are detected.

### Phase 26: JS eval sink guardrail expansion (done)
Goal: cover additional eval-equivalent sinks beyond `eval()` and `new Function()`.

Approach:
- Extend the eval sink contract to also forbid `Function()` calls (same semantics as `new Function()`).

Outcomes:
- Contract suite fails if `Function(` appears in production sources (outside of allowlisted fixtures).

### Phase 27: Subprocess sink guardrail expansion (done)
Goal: catch additional common ways of reaching `child_process.exec/execSync` beyond the direct import/require call patterns.

Approach:
- Extend the subprocess sink scanner to detect:
  - Namespace/default `child_process` aliases (e.g. `import * as cp from "child_process"; cp.exec(...)`)
  - CommonJS aliases assigned from `require("child_process")`
  - Destructuring `exec` / `execSync` from `require("child_process")`
- Add focused parsing contracts to lock in these cases and prevent regressions.

Outcomes:
- Contract suite fails if `child_process.exec/execSync` is reachable via alias/namespace/destructuring patterns.

### Phase 28: Timer-string eval sink guardrails (done)
Goal: prevent accidental reintroduction of string-based timer eval sinks.

Approach:
- Extend the eval sink scan to detect:
  - `setTimeout("...")` / `setInterval("...")`
  - `window.setTimeout("...")` / `globalThis.setTimeout("...")` variants
- Add focused parsing contract coverage.

Outcomes:
- Contract suite fails if string-based timer eval sinks appear in production sources.

### Phase 29: Sink scanner helper consolidation (done)
Goal: reduce drift between security sink scanners by sharing a tiny “parse JS around a match” utility layer.

Approach:
- Add `tests/_helpers/js_scan_parse_helpers.js` for common utilities:
  - whitespace/comment skipping
  - parsing quoted string literals
- Refactor sink scanners to use the shared helper.

Outcomes:
- Sink scanner helpers share one implementation for basic parsing primitives, reducing future drift.

### Phase 30: DOM XSS sink guardrail completeness (done)
Goal: ensure DOM sink guardrails catch bracket-notation access (`obj["innerHTML"]`) in addition to dot access.

Approach:
- Add a shared DOM XSS sink scanner helper that:
  - uses masked code to locate bracket expressions outside strings/comments/regex
  - parses the string literal property name from source
- Refactor the DOM XSS contract to use the shared helper and add focused parsing contract coverage.

Outcomes:
- Contract suite fails if bracket-notation DOM HTML injection sinks appear in production sources.

### Phase 31: Bracket-notation eval/timer sink coverage (done)
Goal: prevent bypassing eval/timer sink guardrails via bracket notation (e.g. `globalThis["eval"]`).

Approach:
- Extend eval sink scanning to detect:
  - `globalThis["eval"](...)` / `window["eval"](...)`
  - `globalThis["setTimeout"]("...")` / `window["setInterval"]("...")` (string first-arg only)
- Add focused parsing contract coverage.

Outcomes:
- Contract suite fails if bracket-notation global eval/timer sinks appear in production sources.

### Phase 32: Sink scanner string-literal hardening (done)
Goal: prevent bypassing sink scanners via JS string literal escape tricks while keeping scans conservative (no full parser).

Approach:
- Harden the shared string-literal parser used by sink scanners to correctly interpret:
  - common escape sequences (`\xNN`, `\uNNNN`, `\u{...}`, line continuations)
  - no-substitution template literals (`` `...` ``) for bracket keys / dynamic import specifiers
  - JS line terminators (LF/CR/CRLF and U+2028/U+2029) in quoted strings and line-comment termination
- Add focused contract coverage to lock in parsing behavior and real bypass cases.

Outcomes:
- Contract suite catches escaped and no-subst-template variants like:
  - `document["wr\u0069te"]`, `globalThis[\`eval\`]`, `require("child\x5fprocess")`
- Contract suite treats U+2028/U+2029 as line terminators for both the source masker and parse helpers.
- Contract suite treats CR/CRLF as line terminators for `//` comments and line numbering.
- Shared parse helper behavior is guarded by contract tests to prevent drift.

### Phase 33: DOM bracket-sink false-positive hardening (done)
Goal: keep the DOM XSS guardrail high-signal by reducing obvious false positives from array literals without weakening real sink detection.

Approach:
- Keep bracket-notation sink detection focused on computed property access like `obj["innerHTML"]`.
- Avoid flagging array literals that merely contain sink-like strings, including common statement shapes:
  - `return ["innerHTML"]`
  - `if (x) ["innerHTML"];` (and similar control-flow headers)
- Lock in the expected behavior with focused contract coverage.

Outcomes:
- Contract suite does not fail on array literals containing sink-like strings.
- Contract suite still fails on computed-property sinks like `el["innerHTML"] = ...`.

### Phase 34: Subprocess sink reference-taking hardening (done)
Goal: prevent bypassing the subprocess sink guardrail by taking references to `child_process.exec/execSync` without calling them inline.

Approach:
- Extend the subprocess sink scanner to flag:
  - property access via child_process aliases (e.g. `cp.exec`, `cp["execSync"]`)
  - property access via direct `require("child_process").exec` / `["execSync"]`
- Keep detection scoped to confirmed `child_process` namespaces/specifiers to avoid false positives.
- Add focused contract coverage.

Outcomes:
- Contract suite fails if `child_process.exec/execSync` is reachable via reference-taking patterns, not just calls/destructuring.

### Phase 35: Awaited dynamic import subprocess sink closure (done)
Goal: prevent bypassing the subprocess sink guardrail via awaited dynamic import member access (e.g. `(await import("child_process")).execSync(...)`).

Approach:
- Extend the subprocess sink scanner to detect `await import("child_process")` followed by:
  - direct member access (dot or bracket), including optional chaining
  - `default` hop patterns where Node ESM interop exposes `child_process` as `module.default`
- Add focused parsing contract coverage for the bypass shapes.

Outcomes:
- Contract suite fails if `child_process.exec/execSync` is reachable via awaited dynamic import member access.

### Phase 36: Unicode-escaped identifier sink hardening (done)
Goal: prevent bypassing sink guardrails via unicode escapes in identifier names (e.g. `document.wr\u0069te(...)`, `globalThis.e\u0076al(...)`, `cp.e\u0078ec(...)`).

Approach:
- Add a conservative identifier parser for `\uXXXX` / `\u{...}` escapes (ASCII-only) for use in sink scanners (no full JS parser).
- Extend sink scanners to detect unicode-escaped identifier forms for:
  - DOM dot-access sinks (`innerHTML`, `outerHTML`, `insertAdjacentHTML`, `createContextualFragment`, and `document.write/writeln`)
  - global eval/timer sinks (`globalThis.e\u0076al`, `window.setTime\u006fut("...")`)
  - subprocess sinks (`cp.e\u0078ec`, `require("child_process").e\u0078ecSync`)
- Add focused parsing contracts for these bypass shapes.

Outcomes:
- Contract suite catches the unicode-escaped identifier bypass patterns above without expanding into a full JS lexer/parser.

### Phase 37: Eval/Function reference-taking hardening (done)
Goal: prevent bypassing eval-sink guardrails via taking references to global eval/Function (e.g. `const e = globalThis["eval"]`, `globalThis["Function"]("...")`).

Approach:
- Extend eval sink scanning to treat global-object dot/bracket access of:
  - `eval`
  - `Function`
  as sinks even when not immediately called (reference-taking), including unicode-escaped identifiers and escaped bracket-string properties.
- Add focused parsing contract coverage to lock in these bypass shapes.

Outcomes:
- Contract suite catches `globalThis.eval`, `globalThis["eval"]`, `globalThis["Function"]`, and unicode-escaped variants.

### Phase 38: Unicode-escaped direct eval/timer identifier hardening (done)
Goal: prevent bypassing eval/timer guardrails via unicode-escaped **direct** identifiers (e.g. `ev\u0061l(...)`, `setTime\u006fut("...")`).

Approach:
- Extend eval sink scanning to detect unicode-escaped direct identifier calls for:
  - `eval(...)`
  - `Function(...)`
  - `setTimeout("...")` / `setInterval("...")` (string first arg only)
- Keep it conservative: only treat direct identifiers (not `.prop` access) and only when the raw identifier text includes a `\u` escape.
- Add focused parsing contract coverage for these cases.

Outcomes:
- Contract suite catches direct-call unicode-escape bypasses without broadening the scanner into a full JS parser.

### Phase 39: Timer optional-call hardening (done)
Goal: prevent bypassing timer-string eval guardrails via optional-call syntax (e.g. `setTimeout?.("...")`, `window.setTimeout?.("...")`).

Approach:
- Update timer-string sink scanning to use `isOptionalCallStart` so both `(... )` and `?.( ... )` forms are recognized.
- Keep the guardrail conservative by requiring a literal string/template first argument.
- Add focused contract coverage for optional-call variants.

Outcomes:
- Contract suite catches `setTimeout?.("...")`, `setInterval?.("...")`, and `window.setTimeout?.("...")` string-timer sinks.

### Phase 40: HTTP error reflection guardrail (done)
Goal: prevent accidental reintroduction of client-visible error reflection in HTTP response bodies (large/untrusted `err.message` values leaking to clients or bloating logs).

Approach:
- Add a contract test that scans production JS/TS sources for high-risk response patterns:
  - `res.send(err.message)` / `reply.send(err.message)`
  - `res.end(String(err))` / `reply.end(String(error))`
  - `socket.end(err.message)` in ad-hoc HTTP responders
- Keep the scan conservative (only `err`/`error` variables; ignore strings/comments).
- Add a focused parsing contract test to lock in scanner correctness (detect sinks, avoid false positives from nearby `catch (err)` blocks).

Outcomes:
- Contract suite fails if new direct `err.message` / `String(err)` response-body reflection patterns appear in production sources.
- Scanner is statement-local: it inspects the matched call’s argument expression (parentheses-balanced) to avoid false positives from nearby `catch (err)` blocks.
- The guardrail also flags status-line reflection via `res.writeHead(status, err.message)` and JSON-body reflection via `JSON.stringify(err)` passed directly to response writers.
- The guardrail is chain-aware: it also covers fluent forms like `reply.code(...).send(...)` and `reply.raw.end(...)`.

### Phase 41: De-duplicate ESM text helpers (done)
Goal: reduce redundant copies of the ESM (JS) text helpers while keeping behavior stable and test-locked.

Approach:
- Make `server/src/text.js` and `tools/net-proxy-server/src/text.js` re-export from the canonical `src/text.js`.
- Keep the existing text parity/contract tests as the drift guard.

Outcomes:
- Removes large duplicated implementations without changing any public behavior (`npm run test:contracts` remains green).

### Phase 42: HTTP error reflection optional-chain hardening (done)
Goal: prevent bypassing the HTTP error reflection guardrail with optional chaining / bracket access on `err`/`error` (e.g. `res.send(err?.message)`, `reply.send(err?.["message"])`).

Approach:
- Extend HTTP response sink scanning to treat `err?.message` and `err?.["message"]` as equivalent to `err.message` when used in response writers.
- Add focused parsing contract coverage for these bypass shapes.

Outcomes:
- Contract suite fails if new optional-chaining / bracket-access error-message reflection patterns appear in production sources.

### Phase 43: HTTP error reflection response-method bracket hardening (done)
Goal: prevent bypassing the HTTP error reflection guardrail via bracket-notation response methods (e.g. `res["send"](err.message)`, `reply?.["send"]?.(err.message)`).

Approach:
- Extend the chain-aware response-call scanner to parse bracket string properties in receiver chains, so `res["send"](...)` and `reply["json"](...)` are scanned like `res.send(...)`.
- Add focused parsing contract coverage for bracket-method variants.

Outcomes:
- Contract suite catches bracket-method response sink bypasses without broadening into full dataflow.

### Phase 44: De-duplicate browser public text helpers (done)
Goal: remove duplicated `formatOneLineError` / UTF-8 one-line formatting helpers in `web/public` scripts while preserving behavior (and keeping CSP fixtures intact).

Approach:
- Add a small shared ESM module in `apps/web/public/_shared/` for one-line UTF-8 formatting and safe error-to-message conversion (optionally including `err.name` fallback).
- Replace duplicated implementations in:
  - `apps/web/public/wasm-jit-csp/main.js`
  - `apps/web/public/assets/security_headers_worker.js`

Outcomes:
- `web/public` scripts stay self-contained (no bundler required) but no longer carry multiple copies of the same helper logic.

### Phase 45: Snapshot UI integration hygiene (done)
Goal: de-duplicate the ad-hoc snapshot UI script and make it usable as a first-class Vite-served page.

Approach:
- Convert `web/snapshot-ui.js` to use the shared `apps/web/public/_shared/text_one_line.js` helper.
- Add `web/snapshot-ui.html` that wires up the expected DOM and loads the script as a module.
- Extend the shared helper to support a `includeNameFallback: "missing"` mode so we can preserve prior semantics where `err.name` is only used when `.message` is missing (not merely empty).

Outcomes:
- Snapshot UI is now a runnable page (`/snapshot-ui.html`) and no longer carries a private copy of one-line error formatting helpers.

### Phase 46: De-duplicate PoC Node server text helpers (done)
Goal: reduce duplicated one-line error formatting helpers in small Node ESM PoC servers.

Approach:
- For ESM PoC servers, import `formatOneLineError` from the canonical `src/text.js` instead of carrying a local `TextEncoder` + UTF-8 truncation implementation.

Outcomes:
- `poc/browser-memory/server.mjs` now uses `src/text.js` for safe, byte-bounded error messages without duplicating the implementation.

### Phase 47: De-duplicate CJS helper copies (done)
Goal: remove duplicated UTF-8 one-line text helpers across CJS utilities without forcing a full CJS→ESM migration.

Approach:
- Introduce a small shared CJS helper: `scripts/_shared/text_one_line.cjs`.
- Replace local helper copies in:
  - `apps/web/scripts/serve.cjs`
  - `tools/disk-streaming-browser-e2e/src/servers.js`
- Add the new CJS helper to the existing text parity tests to prevent drift.

Outcomes:
- CJS utilities share one implementation for `formatOneLineUtf8`/`formatOneLineError`, and parity tests keep it aligned with the canonical `src/text.js`.

### Phase 48: De-duplicate perf dashboard text helpers (done)
Goal: remove duplicated one-line UTF-8 error formatting helpers from the nightly performance dashboard without breaking the gh-pages artifact.

Approach:
- Convert the dashboard script to ESM and import a shared helper from `apps/web/public/_shared/text_one_line.js`.
- Add a local shim module under `bench/dashboard/_shared/` for repo-local serving, and ensure the perf-nightly workflow copies the canonical helper into the published `dist/perf-dashboard/_shared/` so the artifact remains self-contained.

Outcomes:
- `bench/dashboard/app.js` no longer embeds a local UTF-8 one-line formatting implementation; the published dashboard includes the shared helper file.

### Phase 49: HTTP error reflection parenthesized err hardening (done)
Goal: prevent bypassing the HTTP error reflection guardrail via parenthesized `err`/`error` message access (e.g. `res.send((err).message)`, `res.send((err)["message"])`).

Approach:
- Extend the response-argument scan patterns to also catch parenthesized `err`/`error` message access forms while staying conservative (avoid call-argument false positives like `foo(err).message`).
- Add focused parsing contract coverage for these bypass shapes.

Outcomes:
- Contract suite catches the parenthesized error-message reflection bypass shapes without expanding into full dataflow analysis.

### Phase 50: HTTP error reflection bracket-string correctness (done)
Goal: correctly detect `err["message"]` / `err?.["message"]` style sinks despite the scanner masking string literal contents.

Approach:
- Replace regex-based `["message"]` matching with a small parser-based check that scans only masked (non-string/comment) regions, then parses bracket-string properties from the original source and compares the decoded property value to `"message"`.
- Add focused parsing contract coverage for `err?.['message']`, `err['message']`, and escaped string forms like `err["m\\u0065ssage"]`.

Outcomes:
- Contract suite now genuinely enforces bracket-string error message reflection sinks (including escaped string literals) instead of relying on masked string contents.

### Phase 51: HTTP error reflection dot unicode-escape hardening (done)
Goal: prevent bypassing the HTTP error reflection guardrail with unicode-escaped identifier member access (e.g. `err.m\u0065ssage`, `(err).m\u0065ssage`).

Approach:
- Add a small parser-based check that looks for `err`/`error` followed by `.` (or `?.`) and parses the following identifier with unicode escapes, treating a decoded `"message"` property as a sink when the raw identifier includes a `\` escape.
- Add focused parsing contract coverage for `err.m\u0065ssage`, `err?.m\u0065ssage`, and `(err).m\u0065ssage`.

Outcomes:
- Contract suite catches unicode-escaped `.message` bypass shapes without expanding into full dataflow analysis.

### Phase 52: HTTP error reflection unicode optional-chain correctness (done)
Goal: ensure the unicode-escaped dot-property guardrail truly covers optional-chaining `?.` member access (e.g. `err?.m\u0065ssage`, `(err)?.m\u0065ssage`).

Approach:
- Fix the parser-based unicode-dot check to treat `?.` as including the dot (parse the identifier immediately after `?.`).
- Add a focused parsing contract test that isolates unicode-escaped dot-access sinks and asserts they are detected.

Outcomes:
- Contract suite now genuinely enforces unicode-escaped `.message` sinks for both `.` and `?.` forms.

### Phase 53: HTTP error reflection parenthesized direct-arg hardening (done)
Goal: prevent bypassing the HTTP error reflection guardrail via parenthesized direct `err`/`error` arguments (e.g. `reply.raw.end((err))`).

Approach:
- Extend direct-argument parsing to allow harmless grouping parentheses around `err`/`error` while staying statement-local.
- Add focused parsing contract coverage for `reply.raw.end((err))`.

Outcomes:
- Contract suite catches parenthesized direct-err response sinks without expanding into general expression dataflow.

### Phase 54: Text helper drift guard for `web/public` (done)
Goal: keep the browser-public helper `apps/web/public/_shared/text_one_line.js` behavior aligned with the canonical `src/text.js`, preventing silent drift.

Approach:
- Add parity coverage that compares `formatOneLineUtf8` and default `formatOneLineError` outputs against `src/text.js`.
- Add contract coverage for `includeNameFallback` option modes used by public scripts.

Outcomes:
- Contract suite fails if `apps/web/public/_shared/text_one_line.js` formatting diverges from the canonical semantics.

### Phase 55: HTTP error reflection parenthesized String/JSON args (done)
Goal: prevent bypassing HTTP error reflection guardrails via grouping parentheses in `String(...)` / `JSON.stringify(...)` (e.g. `res.send(String((err)))`, `res.send(JSON.stringify((err)))`).

Approach:
- Extend the existing `String(err)` / `JSON.stringify(err)` patterns to accept grouped `(err)`/`((err))` forms.
- Add focused parsing contract coverage for these bypass shapes.

Outcomes:
- Contract suite catches parenthesized `String((err))` and `JSON.stringify((err))` reflection patterns.

### Phase 56: Unicode-brace escape drift guards (done)
Goal: lock in brace-form unicode escape support (`\u{...}`) for identifier parsing and HTTP error reflection sink shapes.

Approach:
- Extend `js_scan_parse_helpers_contract` to cover `parseIdentifierWithUnicodeEscapes` with `\u{...}` (plus an out-of-range rejection case).
- Extend the HTTP error reflection parsing contract to cover `err["m\u{65}ssage"]` and `err.m\u{65}ssage` shapes (and `?.` / parenthesized variants).

Outcomes:
- Contract suite fails if brace-form escape decoding regresses, preventing bypass drift across scanners that rely on these parsing primitives.

### Phase 57: Brace-form unicode escapes end-to-end scanner coverage (done)
Goal: ensure sink scanners themselves remain robust to brace-form escapes (`\u{...}`), not just the shared parsing primitives.

Approach:
- Extend eval sink parsing contract to include brace-form escaped identifiers (e.g. `ev\u{61}l(...)`, `setTime\u{6f}ut("...")`).
- Extend DOM XSS parsing contract to include brace-form escaped sink properties (e.g. `el.inn\u{65}rHTML = ...`, `document.wr\u{69}te(...)`).
- Extend subprocess sink parsing contract to include brace-form escaped `exec`/`execSync` properties (e.g. `cp.e\u{78}ec(...)`).

Outcomes:
- Contract suite will fail if scanner-level detection regresses for `\u{...}` escape bypass shapes.

### Phase 58: Close-race crash hardening sweep (done)
Goal: prevent process crashes from synchronous throws in network send/close paths (WebSocket, streams, and `net.Socket`) under close races.

Approach:
- **Browser WebSocket**:
  - Centralize safe wrappers in `apps/web/src/net/wsSafe.ts` (`wsSendSafe`, `wsCloseSafe`).
  - Use these helpers across the web networking clients so `WebSocket.send()` / `WebSocket.close()` cannot throw and abort the worker/UI.
- **WebRTC DataChannel**:
  - Centralize best-effort wrappers in `apps/web/src/net/rtcSafe.ts` (`dcSendSafe`, `dcCloseSafe`, `pcCloseSafe`).
  - Treat `RTCDataChannel.send()` as a potentially-throwing operation; use `dcSendSafe` in WebRTC transports and surface failures via existing tunnel/proxy error channels (then close deterministically).
- **Node WebSocket + sockets**:
  - Wrap `ws.send(...)`, `socket.write(...)`, `socket.end(...)`, and `socket.destroy()` in `try/catch` where close races can occur.
  - On sync send/write failure, prefer deterministic teardown (`destroy`) and stable, byte-bounded logging rather than reflecting raw error details to clients.
  - Prefer the shared best-effort helpers in `src/ws_safe.js` (`wsSendSafe`, `wsCloseSafe`) to avoid duplicating close-race guards. (`wsCloseSafe` formats close reasons as one-line UTF-8 and caps them to 123 bytes by default per RFC6455.) For convenience, `scripts/_shared/ws_safe.js` re-exports these helpers.
  - When using `wsSendSafe(ws, data, cb)`, be careful with API differences: browser `WebSocket.send(data)` does **not** accept a callback argument. The shared helper must not pass `cb` to 1-arg `send()` implementations (treat callback as a post-send notification only).
    - Note: some ws-style implementations accept callbacks but expose a rest-arg `send(data, ...args)` signature (arity 1). Don’t key solely off `send.length`; use a ws-style indicator (e.g. `.terminate()`) and lock in the heuristic with a contract test.
  - When using stream wrappers like `ws.createWebSocketStream(...)`, do **not** destroy the wrapper stream before sending a close frame (it can prevent the close control frame/code from reaching the peer). Prefer: send close → then destroy on `close`/`error` events (best-effort, try/catch).
  - When implementing raw-upgrade WebSocket protocols (writing frames directly to a `Duplex`), avoid destroying the upgrade socket immediately after writing/echoing a close frame: use `end()` and only `destroy()` after a short timeout so the close response has a chance to flush.
- For raw HTTP upgrade **rejections** (writing `HTTP/1.1 <status>` to a `Duplex`), prefer:
  - `encodeHttpTextResponse` from `src/http_text_response.js` to build the response bytes with a correct `Content-Length` + single `\r\n\r\n` delimiter (and to reject CR/LF in header fields).
  - `endThenDestroyQuietly` from `src/socket_end_then_destroy.js` to ensure the rejection response flushes and the socket doesn’t linger forever.

Outcomes:
- Network relay/upgrade paths and supporting scripts are resilient to close-race sync throws across Node and browser runtimes.
- The contract suite and targeted package test suites were used to validate refactor slices as they landed.

### Phase 59: HTTP streaming pipeline hygiene sweep (done)
Goal: avoid brittle `Readable.pipe(res)` error/teardown behavior by using `pipeline(...)` and making abort/disconnect handling deterministic and non-noisy.

Approach:
- Use `pipeline(stream, res)` (from `node:stream/promises`) for file-to-response streaming paths so backpressure and errors are handled consistently.
- Avoid writing response headers before the stream has actually opened when we need `Content-Length` / `Content-Type` (wait for the stream `open` event first).
- Treat common client abort errors as expected (`ERR_STREAM_PREMATURE_CLOSE`, `ECONNRESET`, `EPIPE`) and suppress them from error logs while still logging real stream failures.

Outcomes:
- Production server static file streaming uses `pipeline(...)` with best-effort teardown and reduced log noise on client disconnects.
- Dev helper servers that stream files over HTTP were updated to use `pipeline(...)`, best-effort `res.destroy()` in error paths, and to suppress noisy expected client abort errors.

### Phase 60: Node WebSocket send helper arity hardening (done)
Goal: avoid accidental callback misuse across ws-style and browser-style WebSocket implementations while preserving error reporting for ws-style `send(..., cb)` APIs.

Approach:
- Keep a shared Node-only helper in `src/ws_safe.js` (`wsSendSafe`, `wsCloseSafe`) for tools/tests/prototypes (re-exported by `scripts/_shared/ws_safe.js`).
- For `wsSendSafe(ws, data, cb)`:
  - Do **not** pass callbacks into browser-style `WebSocket.send(data)` (no callback parameter).
  - Do pass callbacks into ws-style implementations, including those that expose rest-arg `send(data, ...args)` signatures (arity 1) while still accepting callbacks.
- Lock in the behavior with a small contract test (`tests/ws_safe_contract.test.js`).

Outcomes:
- Callback behavior is deterministic across ws-style (`ws`, `tools/minimal_ws.js`) and browser-style WebSocket surfaces.
- Contract suite prevents regressions in the callback arity heuristics.
- The Node-side helpers are robust to `null` / invalid inputs (no “safe helper crashed” footguns); contract tests lock in the behavior.
- `wsCloseSafe` avoids sending empty close reasons and bounds/sanitizes close reasons by default (one-line, UTF-8, 123 bytes).

### Phase 61: ws-shim close-race hardening (done)
Goal: prevent close-race synchronous throws in the `ws` fallback shim from crashing contract tests / tools (especially when exceptions occur inside `socket.write(...)` completion callbacks).

Approach:
- Wrap upgrade handshake `socket.write(...)` in best-effort `try/catch` and bail out on sync failure.
- Ensure `end()` / `destroy()` calls made from inside `socket.write(..., cb)` callbacks are also best-effort (`try/catch`) so exceptions in completion handlers cannot abort the process.
- Avoid relying on direct `socket.write/end/destroy` (and internal `res.writeHead/end/destroy`) method getter reads by fetching methods via `tryGetProp` before invoking (covers hostile/monkeypatched getter sync-throws).

Outcomes:
- `scripts/ws-shim.mjs` is resilient to close races during handshake and close-control-frame mirroring, and does not crash on hostile method getters, matching the broader “no sync-throw on send/close” posture.

### Phase 62: Long-tail boundary hygiene sweep (completed)
Goal: keep shrinking the remaining “long tail” of Node-side boundary surfaces that can synchronously throw (HTTP response writes and socket sends) so non-core tooling can’t crash the process in edge cases (close races, poisoned globals, hostile getters/mocks).

Approach:
- Prefer small, local refactors that preserve behavior while making boundary writes best-effort:
  - wrap `res.writeHead`/`res.setHeader`/`res.end` in `try/catch` with `res.destroy()` fallback
  - wrap `socket.write`/`socket.end` similarly (or reuse existing safe helpers when appropriate)
- Use fast, cheap validation:
  - `npm run test:contracts` for cross-repo guardrails
  - `node --check <file>` for standalone scripts that aren’t exercised by tests

Progress:
- Web + tooling HTTP servers:
  - `apps/web/demo/server.js`: hardened response/header writes with safe helpers + destroy fallback.
  - `web/serve-smoke-test.mjs`: hardened response/header writes with safe helpers + destroy fallback.
  - `web/vite.config.ts`: hardened dev/preview middlewares (`res.setHeader/res.end`) with destroy fallback.
  - `bench/server.js`, `bench/gpu_bench.ts`: hardened response header/body writes against close-race sync throws.
- Aero Gateway (Node):
  - `services/gateway/src/routes/*`: guarded long-tail socket methods (`setNoDelay`, timeouts, `write/end/destroy`) in upgrade/bridge handlers.
  - `services/gateway/bench/run.mjs`: hardened UDP `socket.send(...)` and TCP sink ACK `socket.write/end` paths against sync throws and repeated writes.
  - `services/gateway/src/dns/upstream.ts`: hardened `queryUdpUpstream` against sync `send()` throws and added focused regression coverage.
- Net proxy + related tools (Node):
  - `services/net-proxy/src/*`: hardened upgrade/relay boundaries (hostile getters + send/close races) and tightened UDP send failure behavior (fatal close in multiplexed mode, with regression coverage).
  - `tools/net-proxy-server/src/server.js`: guarded pause/resume and backpressure paths to avoid sync-throw crashes in less-traveled tooling.
  - `tools/disk-streaming-browser-e2e/src/servers.js`, `tools/minimal_ws.js`, `scripts/ws-shim.mjs`, `scripts/ci/run_browser_perf.mjs`: hardened long-tail `res.*` / `req.end()` / socket init / `.unref()` boundaries while preserving behavior.
- Shared teardown helper:
  - `src/socket_end_then_destroy.{js,cjs}`: hardened against hostile/monkeypatched method getters (`end/destroy/once/off/removeListener/unref`) and extended contract coverage to lock in “never throw” behavior.
- Shared timer helper:
  - `src/unref_safe.js`: added `unrefBestEffort(...)` so we don’t rely on `timer.unref?.()` (which still reads the getter and can synchronously throw). Refactored `src/` call sites and added contract coverage.
  - Follow-up: refactored additional Node-side packages (gateway/proxy/tools) to use `unrefBestEffort` for timers (connect timeouts, GC intervals, test timeouts) to keep the hostile-getter posture consistent outside `src/`.
- Guardrails:
  - `tests/unref_usage_contract.test.js`: prevents reintroducing direct `.unref()` calls in production sources (prefer `.unref?.()`).
  - `tests/dgram_usage_contract.test.js`: restricts `dgram.createSocket(...)` usage to known modules.

### Phase 63: Post-sweep type-safety polish (done)
Goal: after the heavy boundary hardening sweeps, do small follow-up refactors that keep the new guards **type-safe** and avoid “`any` + repetition” patterns that can rot over time.

Approach:
- Prefer local helpers that reduce repeated try/catch boilerplate without changing behavior.
- Keep middleware signatures accurate (`http.IncomingMessage` / `http.ServerResponse`) so future edits don’t reintroduce unsafe assumptions.
- Validate with cheap checks (`npm -w <pkg> run typecheck`, `npm run test:contracts`).

Progress:
- `web/vite.config.ts`: replaced ad-hoc `(res as any).destroy?.()` fallbacks with a typed `destroyResponseQuietly` helper and typed middleware signatures.

### Phase 64: Close long-tail helper drift gaps (done)
Goal: where we deliberately keep multiple copies of “safe boundary” helpers (ESM vs CJS workspaces), ensure their high-risk behavior doesn’t silently drift.

Approach:
- Prefer **package-local tests** over cross-workspace imports (avoids ESM/CJS tooling friction).
- Mirror the “high signal” contract expectations from repo-root `ws_safe` tests inside the CJS packages that carry their own implementations.

Progress:
- `services/net-proxy/src/wsClose.ts`: added missing unit coverage for `wsSendSafe` arity heuristics in `net-proxy`’s own test suite.
- `services/net-proxy/src/wsClose.ts`: aligned `wsIsOpenSafe` “fail open when readyState is not observable” semantics with the canonical `src/ws_safe.js` behavior (with local tests).

### Phase 65: Net-proxy wsClose helper completeness (done)
Goal: ensure the CJS `net-proxy` copies of the WebSocket safety helpers have the same “high signal” defensive behavior as the canonical repo implementation.

Progress:
- `services/net-proxy/src/test/ws-close.test.ts`: added missing coverage for:
  - `wsCloseSafe` empty reason behavior (treat as absent)
  - `wsCloseSafe` hostile reason input (toString throws)
  - `wsCloseSafe` invalid ws input (no-op)
  - `wsSendSafe` invalid ws input (returns false, cb async)
  - `wsIsOpenSafe` invalid ws input + OPEN getter throw behavior

### Phase 66: Clamp hostile negative bufferedAmount (done)
Goal: treat negative `bufferedAmount` readings as invalid and clamp them to `0` so hostile/buggy implementations cannot undercount backlog and bypass backpressure.

Progress:
- `src/ws_backpressure.js`: clamp negative `ws.bufferedAmount` to `0`.
- `apps/web/src/net/{wsSafe.ts,rtcSafe.ts}`: clamp negative `bufferedAmount` to `0`.
- `services/net-proxy/src/wsBufferedAmount.ts`: clamp negative `bufferedAmount` to `0`.
- Tests:
  - `tests/ws_backpressure_contract.test.js`: added regression for negative bufferedAmount.
  - `tests/web_ws_safe_contract.test.js`: added `wsBufferedAmountSafe` regression (incl. negative).
  - `tests/web_rtc_safe_contract.test.js`: extended regression to include negative.
  - `services/net-proxy/src/test/ws-buffered-amount.test.ts`: added negative bufferedAmount coverage.

### Phase 67: De-duplicate socket/stream safe method helpers (done)
Goal: reduce drift and repetition by single-sourcing the “hostile getter + close-race safe” socket/stream method invocations used across Node packages.

Approach:
- Add canonical helpers in repo-root `src/` with ESM+CJS parity:
  - `src/socket_safe.js`, `src/socket_safe.cjs` (plus `.d.ts` stubs)
- Convert workspace-local helpers into thin re-exports:
  - `services/net-proxy/src/socketSafe.ts` → re-export from `src/socket_safe.cjs`
  - `services/gateway/src/routes/socketSafe.ts` → re-export from `src/socket_safe.js` (keep `destroyQuietly` as a stable local alias)
- Add contract/parity coverage:
  - `tests/socket_safe_contract.test.js`
  - `tests/socket_safe_parity.test.js`

Outcomes:
- Socket/stream best-effort calls (`destroy/end/pause/resume/...`) now share one implementation across Node workspaces, making behavior changes easier to audit and test-lock.

### Phase 68: De-duplicate HTTP response write helpers (done)
Goal: reduce drift by single-sourcing the “best-effort writeHead/end with destroy fallback” pattern used by Node HTTP servers.

Approach:
- Add canonical helpers with ESM+CJS parity:
  - `src/http_response_safe.js`, `src/http_response_safe.cjs` (plus `.d.ts` stubs)
- Convert workspace-local helper into a thin re-export:
  - `services/net-proxy/src/httpResponseSafe.ts` → re-export from `src/http_response_safe.cjs`
- Add contract/parity coverage:
  - `tests/http_response_safe_contract.test.js`
  - `tests/http_response_safe_parity.test.js`

Outcomes:
- Node HTTP responder helpers are single-sourced and guarded by the contract suite, preventing subtle drift across packages.

### Phase 69: De-duplicate web helper server response writes (done)
Goal: reduce drift and boilerplate in the repo’s small Node “web helper servers” by reusing the canonical best-effort HTTP response writer.

Approach:
- Refactor simple web servers to use `tryWriteResponse` from `src/http_response_safe.js` instead of local `tryGetProp` + `writeHead/end/destroy` wrappers:
  - `apps/web/demo/server.js`
  - `web/serve-smoke-test.mjs`
- Preserve behavior by keeping existing headers/caching semantics; only centralize the write path and its error handling.

Outcomes:
- Web helper servers have simpler, more consistent response writing and inherit the same hostile-getter/close-race hardening as the rest of the repo.

### Phase 70: De-duplicate web tooling response safety helpers (done)
Goal: reduce drift by removing ad-hoc “destroy on response write failure” helpers in web tooling and reusing the canonical socket/HTTP helpers.

Approach:
- `web/vite.config.ts`: replace `destroyResponseQuietly` (and its `tryGetProp` dependency) with `destroyBestEffort` from `src/socket_safe.js`.
- `apps/web/scripts/serve.cjs`: replace local `writeHeadSafe/endSafe/destroySafe` helpers with `tryWriteResponse` from `src/http_response_safe.cjs`.

Outcomes:
- Web tooling now uses the same canonical safety helpers as production services, shrinking duplicated “best-effort response” implementations.

### Phase 71: De-duplicate bench server response safety helpers (done)
Goal: reduce drift in bench tooling by reusing the canonical “destroy best-effort” and “tryWriteResponse” primitives for non-streaming responses.

Approach:
- `bench/server.js`:
  - Replace local `destroyResponseQuietly` with `destroyBestEffort` from `src/socket_safe.js`.
  - Refactor `sendText(...)` to use `tryWriteResponse` from `src/http_response_safe.js` (preserving existing headers like `Cache-Control: no-store` and bounded body formatting).

Outcomes:
- Bench’s dev-only static server now shares the same hardened response teardown/write path as production Node services, without changing the streaming file path (`pipeline`).

### Phase 72: De-duplicate GPU bench server response writes (done)
Goal: reduce drift in bench tooling by removing ad-hoc response “destroy on failure” wrappers and reusing the canonical helpers.

Approach:
- `bench/gpu_bench.ts`:
  - Replace local `destroyResponseQuietly` wrappers with `destroyBestEffort` from `src/socket_safe.js`.
  - Serve the bench HTML using `tryWriteResponse` from `src/http_response_safe.js` (instead of `setHeaderBestEffort` + `endBestEffort`).

Outcomes:
- Bench tooling uses the same hardened HTTP response writer and destroy semantics as the rest of the repo, shrinking duplicated “best-effort response” logic.

### Phase 73: De-duplicate dev server end() safety wrappers (done)
Goal: reduce drift in small Node dev helper servers by removing local “end + destroy-on-throw” wrappers and reusing the canonical response writer.

Approach:
- `server/range_server.js`: replace local `endSafe` with `tryWriteResponse(..., headers=null)` in all non-streaming response paths (OPTIONS / HEAD / 304 / errors).
- `server/chunk_server.js`: same replacement for non-streaming response paths.

Outcomes:
- Dev helper servers now share the same hardened “writeHead + end with destroy fallback” behavior as production Node services, while keeping their streaming `pipeline(...)` paths unchanged.

### Phase 74: Remove remaining bench server response helper drift (done)
Goal: finish de-duplicating bench tooling response boundaries by removing remaining local `tryGetProp`-based `setHeader/end` wrappers.

Approach:
- `bench/server.js`:
  - Remove local `setHeaderBestEffort` / `endBestEffort` wrappers and stop importing `src/safe_props.js` here.
  - Use `tryWriteResponse` for all non-streaming responses (e.g. OPTIONS / HEAD / 405 errors).
  - Use a single `res.writeHead(200, headers)` + `pipeline(...)` for streaming file responses.

Outcomes:
- Bench static server now has no ad-hoc response-method wrappers; it uses the same canonical best-effort HTTP writer as the rest of the repo and keeps the streaming path minimal and explicit.

### Phase 75: De-duplicate “call a socket method and capture error” helpers (done)
Goal: reduce drift by single-sourcing the “call method if present; return thrown error (or missing-method error) instead of throwing” helper used by TCP backpressure/teardown paths.

Approach:
- Extend canonical helpers:
  - `src/socket_safe.js` / `src/socket_safe.cjs`: add `callMethodCaptureErrorBestEffort(obj, key, ...args)` with ESM/CJS parity + `.d.ts` declarations.
  - Add contract/parity coverage:
    - `tests/socket_safe_contract.test.js`
    - `tests/socket_safe_parity.test.js`
- Migrate callsites:
  - `server/src/tcpProxy.js`: remove local `tryGetMethod`/`destroyBestEffort`/`callMethodCaptureError` implementations; reuse canonical helpers.
  - `tools/net-proxy-server/src/server.js`: remove local `tryGetMethod`/`callMethod*`/`destroyQuietly`/`removeAllListenersQuietly` and reuse canonical helpers.

Outcomes:
- TCP pause/resume/end error capture is now consistent across Node packages and locked by the repo-root contract suite.

### Phase 76: De-duplicate disk-streaming browser e2e server response helpers (done)
Goal: reduce drift in browser e2e harness servers by removing local response “destroy on failure” wrappers and reusing the canonical HTTP response writer.

Approach:
- `tools/disk-streaming-browser-e2e/src/servers.js`:
  - Remove local `destroyQuietly` / `setHeaderBestEffort` / `endBestEffort` / `withCommonAppHeaders` wrappers.
  - Use `tryWriteResponse` from `src/http_response_safe.cjs` and a shared `SAB_HEADERS` constant so all responses consistently include COOP/COEP for `crossOriginIsolated`.

Outcomes:
- The browser e2e harness server now uses the same hardened response writer as production Node services, shrinking duplicated “best-effort response” logic.

### Phase 77: De-duplicate ws-shim socket method wrappers (done)
Goal: reduce drift in the `ws` fallback shim by reusing canonical “destroy/end best-effort” and “call method + capture error” helpers.

Approach:
- `scripts/ws-shim.mjs`:
  - Remove local `tryGetProp` + `tryGetMethod` + `callMethodOptional` helpers.
  - Replace local `destroyQuietly` / `endQuietly` / `writeCaptureError` / `callRequiredMethodCaptureError` logic with imports from `src/socket_safe.js`:
    - `destroyBestEffort`
    - `endBestEffort`
    - `callMethodCaptureErrorBestEffort`

Outcomes:
- The shim’s close-race/hostile-getter-safe socket method calls are now single-sourced, and the contract suite continues to validate shim behavior.

### Phase 78: De-duplicate socket_end_then_destroy internal helpers (done)
Goal: reduce drift by removing duplicated “safe method lookup + optional invocation” logic from the canonical `socket_end_then_destroy` helper itself.

Approach:
- `src/socket_safe.{js,cjs}`:
  - Export `tryGetMethodBestEffort(obj, key)` for safe method retrieval.
  - Export `callMethodBestEffort(obj, key, ...args)` for safe optional invocations (returns boolean).
  - Update `.d.ts` stubs accordingly.
- `src/socket_end_then_destroy.{js,cjs}`:
  - Replace local `tryGetProp`/`tryGetMethod`/`callMethodOptional` helpers with the canonical exports above.
  - Replace ad-hoc `timer.unref` best-effort call with `unrefBestEffort(...)`.

Outcomes:
- `socket_end_then_destroy` is now implemented in terms of the same canonical safety helpers it conceptually depends on, making future behavior changes easier to audit and test-lock.

### Phase 79: Test-lock new socket_safe exports (done)
Goal: prevent drift in newly-exported socket helper primitives (`tryGetMethodBestEffort`, `callMethodBestEffort`) by extending contract/parity coverage.

Approach:
- `tests/socket_safe_contract.test.js`: add coverage for:
  - `tryGetMethodBestEffort` return shape (function or null)
  - `callMethodBestEffort` return behavior (true on missing method, false on thrown method)
- `tests/socket_safe_parity.test.js`: add basic ESM/CJS parity assertions for the new exports.

Outcomes:
- The canonical helper surface is guarded by the contract suite, reducing the chance of subtle ESM/CJS drift when these primitives are reused by other modules.

### Phase 80: De-duplicate raw socket write/destroy fallbacks (done)
Goal: reduce drift by reusing canonical socket safety helpers in remaining “raw socket write” boundaries.

Approach:
- `src/http_upgrade_reject.js`: replace ad-hoc `socket?.destroy?.()` fallback with `destroyBestEffort` from `src/socket_safe.js`.
- `src/ws_handshake_response.js`: replace `try { socket.write(...) } catch { socket.destroy?.() }` with:
  - `callMethodCaptureErrorBestEffort(socket, "write", ...)`
  - `destroyBestEffort(socket)` on error

Outcomes:
- Raw socket handshake/rejection paths now use the same hardened “hostile getter + close-race safe” helpers as the rest of the repo, further reducing ad-hoc best-effort logic.

### Phase 81: Web: De-duplicate best-effort `.destroy?.()` calls (done)
Goal: reduce drift in browser/WebWorker code by centralizing “best-effort optional method call” behavior (and avoid direct optional-chaining method calls that can throw on hostile proxies).

Approach:
- Add `apps/web/src/safeMethod.ts`:
  - `callMethodBestEffort(obj, key, ...args)` for safe, best-effort optional method invocation (returns boolean).
  - `destroyBestEffort(obj)` convenience wrapper.
- Replace ad-hoc `.destroy?.()` / `.unconfigure?.()` calls with the canonical helper:
  - `apps/web/src/gpu/webgpu-presenter.ts`
  - `apps/web/src/gpu/webgpu-presenter-backend.ts`
  - `apps/web/src/workers/gpu-worker.ts`
  - `apps/web/src/workers/io.worker.ts`
  - `apps/web/src/bench/webgpu_bench.ts`

Outcomes:
- WebGPU presenter/worker teardown paths now share a single hardened “call optional method best-effort” primitive, reducing repetition and making future hardening changes one-touch.

### Phase 82: Web: Test-lock and extend safeMethod usage for event callbacks (done)
Goal: make web-side “optional method call” behavior more robust and prevent drift by covering hostile getter cases with tests.

Approach:
- `apps/web/src/safeMethod.ts`:
  - Export `tryGetMethodBestEffort(...)` so callsites can safely detect presence (without treating missing as success).
- Replace remaining high-value optional-chaining method calls in WebGPU paths:
  - Use `callMethodBestEffort(ev, "preventDefault")` instead of `ev?.preventDefault?.()` / casted variants.
  - Use `tryGetMethodBestEffort(device, "addEventListener"/"removeEventListener")` to install/remove `"uncapturederror"` handlers without exposing hostile getter throws.
  - `apps/web/src/bench/webgpu_bench.ts`: simplify teardown by calling `callMethodBestEffort(device, "removeEventListener", ...)`.
- Add contract coverage:
  - `tests/safe_method_web_contract.test.js`: covers hostile getters, missing methods, and throwing methods for `tryGetMethodBestEffort`/`callMethodBestEffort`/`destroyBestEffort`.

Outcomes:
- WebGPU uncaptured-error handler install/remove and `preventDefault` calls are now centralized and hostile-getter-safe, with contract tests guarding future changes.

### Phase 83: Gateway: remove legacy destroyQuietly alias (done)
Goal: reduce drift and improve naming clarity in the gateway routes by using the canonical helper name (`destroyBestEffort`) consistently.

Approach:
- `services/gateway/src/routes/socketSafe.ts`: stop re-exporting `destroyBestEffort` under the legacy alias `destroyQuietly`.
- Update gateway route code to import/call `destroyBestEffort` directly:
  - `tcpProxy.ts`
  - `tcpMuxBridge.ts`
  - `tcpBridge.ts`
  - `wsDuplexClose.ts`

Outcomes:
- Gateway routes now use a single, repo-wide name for best-effort teardown (`destroyBestEffort`), reducing cognitive overhead and avoiding “quietly” vs “best-effort” naming drift.

### Phase 84: De-duplicate “write + capture ok + capture error” patterns (done)
Goal: remove remaining ad-hoc `try/catch` wrappers around `stream.write(...)` in backpressure-sensitive code that needs both:
- the write return value (`ok`) and
- a best-effort error capture path that is safe against hostile getters.

Approach:
- `src/socket_safe.{js,cjs}`:
  - Add `writeCaptureErrorBestEffort(stream, ...args) -> { ok: boolean, err: unknown | null }`.
  - Update `.d.ts` stubs and extend contract/parity tests to lock behavior.
- Replace local `try { ok = x.write(...) } catch { ... }` blocks with the canonical helper:
  - `services/gateway/src/routes/tcpMuxBridge.ts`
  - `services/gateway/src/routes/tcpBridge.ts`
  - `services/net-proxy/src/tcpRelay.ts`
  - `services/net-proxy/src/tcpMuxRelay.ts`
- Extend per-package shim exports where used:
  - `services/gateway/src/routes/socketSafe.ts`
  - `services/net-proxy/src/socketSafe.ts`

Outcomes:
- “write return + sync throw” handling is now single-sourced and test-locked, reducing drift and making backpressure-related safety changes one-touch.

### Phase 85: Apply writeCaptureErrorBestEffort to remaining tool callsites (done)
Goal: remove the last remaining `let ok=false; try { ok = socket.write(...) } catch { ... }` patterns in dev/test tooling so backpressure handling stays consistent across the repo.

Approach:
- `tools/net-proxy-server/src/server.js`:
  - Replace the remaining TCP stream `.write(...)` try/catch blocks with `writeCaptureErrorBestEffort`.

Outcomes:
- Tooling now uses the same canonical “write return + sync throw” primitive as production code, eliminating drift.

### Phase 86: Prefer specialized capture helpers over generic callMethodCaptureErrorBestEffort (done)
Goal: further reduce drift by using more specific, self-documenting primitives (`writeCaptureErrorBestEffort`, `endCaptureErrorBestEffort`) where callsites were still using the generic “call method + capture error” helper.

Approach:
- `src/ws_handshake_response.js`: use `writeCaptureErrorBestEffort` for handshake writes.
- `tools/net-proxy-server/src/server.js`: use `endCaptureErrorBestEffort` for FIN forwarding.

Outcomes:
- Remaining “write/end” capture callsites now share the canonical helpers dedicated to those operations, keeping behavior consistent and intent clearer.

### Phase 87: De-duplicate minimal_ws socket write/destroy wrappers (done)
Goal: reduce drift in `tools/minimal_ws.js` by reusing canonical “safe write capture” + “destroy best-effort” helpers (including hostile getter safety).

Approach:
- `tools/minimal_ws.js`:
  - Implement `trySocketWrite(...)` via `writeCaptureErrorBestEffort(...)` (preserving callback error microtask semantics).
  - Implement `trySocketDestroy(...)` via `destroyBestEffort(...)`.

Outcomes:
- The minimal WebSocket shim now shares the repo-wide hardened socket write/destroy primitives, without changing contract behavior.

### Phase 88: HTTP response safe API alignment (done)
Goal: keep the canonical “best-effort response writer” aligned with Node’s real `ServerResponse.writeHead(...)` overloads while preserving the repo’s “never throw at boundaries” posture.

Approach:
- `src/http_response_safe.{js,cjs}`:
  - Treat `headers` as one of:
    - `OutgoingHttpHeaders` (object map)
    - `OutgoingHttpHeader[]` raw header list (validated; must be even-length and alternating `string` keys with `string | number | string[]` values)
  - Ignore invalid header shapes (including empty arrays) and fall back to `writeHead(statusCode)` instead of throwing and tearing down.
  - Harden `sendJsonNoStore` / `sendTextNoStore` against hostile `opts.contentType` getters and missing/invalid values (stable defaults).
- Typings:
  - Update `src/http_response_safe*.d.ts` to reflect the supported header shapes and optional `contentType`.
- Tests:
  - Extend contract + parity coverage to lock in header-array handling and `opts.contentType` hardening.

Outcomes:
- Canonical response writer supports Node’s `writeHead(status, rawHeadersArray)` path safely and deterministically.
- `sendJsonNoStore` / `sendTextNoStore` no longer rely on trusted `opts` shapes for content-type selection.
- ESM/CJS parity is test-locked for the new behaviors.

### Phase 89: Web helper parity polish (done)
Goal: keep browser/worker best-effort helpers aligned with the canonical “nullish + object/function receiver” posture so hostile proxies and primitive inputs cannot crash teardown paths.

Approach:
- `apps/web/src/unrefSafe.ts`: align the guard to `handle == null` (avoid “falsy” semantics drift).
- `apps/web/src/safeMethod.ts`: bail out early for non-object/non-function receivers.

Outcomes:
- Web helper behavior stays consistent with the canonical Node-side helpers, and contract coverage continues to prevent drift.

### Phase 90: ws_safe invalid-input hardening (done)
Goal: ensure canonical Node-side WebSocket safety helpers fail closed on clearly-invalid inputs while preserving “fail open when state is not observable” semantics for real WebSocket-like objects.

Approach:
- `src/ws_safe.js`:
  - Make `wsIsOpenSafe` return `false` for non-object/non-function inputs (avoid treating primitives as open).
- Tests:
  - Extend `tests/ws_safe_contract.test.js` to lock in invalid-input behavior.

Outcomes:
- `wsIsOpenSafe(123)` (and other primitive inputs) reliably returns `false`.
- Existing behavior for object-like inputs without observable `readyState` remains unchanged and contract-locked.

### Phase 91: ws_backpressure invalid-input hardening + typing alignment (done)
Goal: keep `createWsSendQueue` robust to invalid `ws` inputs and keep its `.d.ts`/JSDoc aligned with its defaulting behavior.

Approach:
- `src/ws_backpressure.js`:
  - Make the internal `isOpen()` fail closed for non-object/non-function `ws` inputs.
  - Update JSDoc to mark `ws` / `highWatermarkBytes` / `lowWatermarkBytes` as optional (runtime defaults).
- `src/ws_backpressure.d.ts`:
  - Make `ws`, `highWatermarkBytes`, and `lowWatermarkBytes` optional.
  - Allow `createWsSendQueue(opts?)` to be called with `opts` omitted, matching runtime behavior.
- Tests:
  - Add a contract regression to ensure primitive `ws` inputs are not treated as open.

Outcomes:
- Backpressure callbacks are not triggered for nonsense primitive `ws` inputs.
- Type declarations reflect the actual supported call shapes (defaults) without affecting runtime behavior.

### Phase 92: Contract-suite drift + timing hardening (done)
Goal: keep the contract/parity suite reliable across environments and prevent “type stub drift” between ESM and CJS helper surfaces.

Approach:
- Add a contract test that auto-discovers `src/**/*.cjs.d.ts` files and enforces:
  - the matching `src/**/*.d.ts` exists, and
  - normalized contents match (ignoring CRLF and trailing whitespace).
- Add a contract test that ensures dual-module helper stubs match the *runtime* export surface:
  - parse `export function ...` declarations from the `.d.ts` files, and
  - assert the same names exist as `function` exports in both `src/<name>.js` (ESM) and `src/<name>.cjs` (CJS).
- Extend the same runtime export contract to cover ESM-only `src/**/*.d.ts` stubs:
  - when there is no `.cjs.d.ts` pair, require a matching `src/<name>.js` runtime module and validate exported function names there.
- Add a module boundary contract that prevents workspaces from deep-importing repo-root `src/*` helpers directly:
  - allow only minimal `export * from ".../src/..."` shim modules inside each workspace.
- Replace brittle fixed sleeps in the `ws_backpressure` contract with deterministic scheduling primitives / bounded waits.

Outcomes:
- The contract suite fails if any dual-module `.d.ts` pair in `src/` drifts.
- The contract suite fails if any helper `.d.ts` declares a function that isn’t exported at runtime (ESM and/or CJS, depending on module format).
- The contract suite fails if `net-proxy` or `aero-gateway` code deep-imports repo-root `src/*` helpers outside of shim modules.
- `ws_backpressure` contract timing is deterministic and less sensitive to runtime load or event-loop scheduling differences.

### Phase 93: Helper typing ergonomics (done)
Goal: make the repo’s defensive helper surfaces feel “type-correct” in TypeScript without changing runtime behavior.

Approach:
- Prefer `PropertyKey` over `string` for helper APIs that accept property/method keys, matching real JS semantics (including `symbol` keys).
- Add small contract coverage for symbol-key method lookup/invocation where helpers explicitly accept a key argument.

Outcomes:
- `safe_props` and `socket_safe` TypeScript stubs accept `PropertyKey` where appropriate.
- Contract suite includes symbol-key regressions for:
  - the web helper (`apps/web/src/safeMethod.ts`)
  - the canonical Node socket helper (`src/socket_safe.js`)
  - the canonical safe getter helper (`src/safe_props.js`)
- `safe_props` runtime guards use the repo-standard “nullish + object/function” posture (`obj == null`) to avoid falsy-vs-nullish drift.

### Phase 94: Landing polish + PR visibility (done)
Goal: make it easy to open PRs and track CI status in environments without GitHub CLI tooling, without weakening CI enforcement or adding manual steps.

Approach:
- `scripts/safe-run.sh`: silence non-fatal Node version mismatch notes by default under Node major mismatch (opt-out via `AERO_CHECK_NODE_QUIET=0`), while keeping enforcement behavior unchanged.
- Add a tiny repo-root helper (`scripts/print-pr-url.mjs`) to print compare/PR URLs (and optionally Actions URLs) for the current branch:
  - support both env and cross-platform CLI flags (`--actions`, `--base`, `--remote`, `--branch`)
  - provide repo-root `npm run pr:url` / `npm run pr:links` shortcuts
  - contract-test the output and script wiring to prevent drift and portability regressions.
- Docs: update `AGENTS.md`, `README.md`, and `CONTRIBUTING.md` to point developers at the canonical commands and to avoid copy/paste footguns.

Outcomes:
- Agent/local runs are quieter by default under Node major mismatch (without changing enforcement behavior).
- PR creation + CI visibility can be done via copy/paste links even when `gh` isn’t available.
- Cross-platform usage is guarded by contracts (no POSIX-only env assignment in npm scripts).

### Phase 95: URL fallback drift guard (done)
Goal: prevent accidental reintroduction of “missing request URL defaults to `/`” patterns that can mask hostile/malformed request objects and silently change routing behavior.

Approach:
- Remove `tryGetProp(req, "url") ?? "/"` / `tryGetStringProp(req, "url") ?? "/"` style fallbacks in request handlers; instead require a real string URL and treat missing/invalid values as a 400.
- Keep upgrade paths that need to differentiate “getter threw” vs “invalid/missing url” using a direct `req.url` read inside an outer `try/catch`, so getter throws remain deterministic 500s where tests expect it.
- Add a contract test that forbids `tryGetProp(<req-ish>, "url") (??| ||) "/"`, `tryGetStringProp(<req-ish>, "url") (??| ||) "/"`, and `(req|_req).url (??| ||) "/"` in production sources.

Outcomes:
- Production sources no longer silently treat a missing/hostile `req.url` as a real `/` request.
- Contract suite fails if the unsafe fallback pattern is reintroduced.

### Phase 96: Forbid direct `new URL(req.url, base)` (done)
Goal: prevent accidentally reintroducing implicit coercion/trimming hazards by passing request URL getters directly into the WHATWG URL constructor.

Approach:
- Add a contract test that forbids `new URL((req|_req).url, base)` in production sources (including parenthesized, optional-chain, and unicode-escaped identifier variants).
- Add a focused parsing contract to lock down the scanner’s match behavior and avoid false positives from strings/comments.

Outcomes:
- Contract suite fails if production code starts parsing request targets via direct `req.url` getters instead of validating and threading a trusted `rawUrl` string.

### Phase 97: Web worker init-message parsing hardening (in progress)
Goal: ensure web worker init message handlers don’t crash on hostile `MessageEvent.data` (proxy/getter throws) and don’t implicitly trust inherited properties.

Approach:
- Centralize init-message parsing in `apps/web/src/workers/worker_init_parsers.ts` using best-effort safe access helpers.
- Refactor small worker entrypoints to parse `ev.data` defensively before touching any fields.
- Add a contract test that the init parsers never throw when given hostile proxy payloads.

Outcomes:
- Worker init parsing is deterministic and resilient to “poisoned” message payloads.

Some coding guidelines:

## General Principles

Write **modern, clean, elegant, concise, and direct** Rust code.

## Code Style

### Indentation
- **2 spaces** for indentation (not 4, not tabs)

### Control Flow
- **Early returns** - avoid nesting at all costs
- Use `for` loops over `.for_each()` unless using rayon for parallelism
- Functional/declarative style with iterators where it's clearer
- Imperative style where it's more readable

### Imports
- **Always import types, never use qualified syntax**
- Import what you use at the top of the file
- Good: `use std::io::Result;` then use `Result`
- Bad: `std::io::Result` in the code
- Good: `use MyModule::{Type1, Type2};` then use `Type1`, `Type2`
- Bad: `MyModule::Type1`, `MyModule::Type2` everywhere

## Rust-Specific Patterns

### Async Closures
- **`async ||` and `async |args|` are legal Rust syntax** - DO NOT rewrite them
- Use `async |item| { ... }` directly, NOT `|item| { async move { ... } }`
- Good: `.for_each_concurrent(n, async |item| { ... })`
- Bad: `.for_each_concurrent(n, |item| { async move { ... } })`

### Iterators
- Use iterator chains for data transformations
- `.collect_vec()` from itertools instead of `.collect::<Vec<_>>()`
- Prefer `.map()`, `.filter()`, `.flat_map()` over manual loops for transformations
- Use rayon's `.par_iter()` for parallelism

### Naming
- Function names should describe what they create/do
  - Good: `rocksdb_options()` (creates options)
  - Bad: `configure_rocksdb()` (sounds like it configures something)

### Type Casts
- Avoid unnecessary type casts
- Question every cast - is it really needed?

### Concurrency and Sharing
- **NEVER use Arc when you don't need it**
  - Rayon closures capture by reference - don't Arc or clone unnecessarily
  - Question every Arc/clone - why is this here?

## Libraries and APIs

### Research and Use Modern APIs
- Always research the current best practices for libraries you're using
- Use the latest stable APIs, not outdated patterns
- Don't assume - look up documentation and examples
- If something seems inefficient, research if there's a better way

## Logging and Observability

### Structured Logging
- Use `tracing` crate, not `println!` for logging
- **Structured logging means using fields**, not string interpolation:
  - Good: `info!(cpus = num_cpus, threads = num_threads, "starting")`
  - Bad: `info!("Starting with {} cpus and {} threads", num_cpus, num_threads)`
- **Use shorthand when variable name matches field name**:
  - Good: `info!(labels_file, count, "appending")`
  - Bad: `info!(labels_file = labels_file, count = count, "appending")`

## Performance

### Parallelism
- Use rayon for CPU-bound parallel operations
- Example: `files.par_iter().map(...).sum()`

## Cargo Commands

### Validation
- **NEVER use `cargo build` or `cargo build --release` just to validate code**
- **NEVER run binaries to test if code works** (see warning at top of this document)
- Use `cargo check` or `cargo check --workspace` for validation - it's 10-100x faster than building

### Prove correctness BEFORE implementing

Before making architectural changes or rewrites:

1. **Write down the algorithm** in plain language
2. **Prove it correct** - trace through edge cases, identify invariants
3. **Only then implement**

If you can't prove it correct on paper, you can't implement it correctly in code.

## When in Doubt

- **PROVE** correctness before implementing
- **Simpler is better** than clever
- **Direct is better** than abstracted
- **Explicit is better** than implicit
- **Fast failures** are better than silent failures

---

## Purged root document: CODE_OF_CONDUCT.md

# Code of Conduct

## Our Pledge

We as members, contributors, and leaders pledge to make participation in our
community a harassment-free experience for everyone, regardless of age, body
size, visible or invisible disability, ethnicity, sex characteristics, gender
identity and expression, level of experience, education, socio-economic
status, nationality, personal appearance, race, religion, or sexual identity
and orientation.

We pledge to act and interact in ways that contribute to an open, welcoming,
diverse, inclusive, and healthy community.

## Our Standards

Examples of behavior that contributes to a positive environment for our
community include:

- Demonstrating empathy and kindness toward other people
- Being respectful of differing opinions, viewpoints, and experiences
- Giving and gracefully accepting constructive feedback
- Accepting responsibility and apologizing to those affected by our mistakes,
  and learning from the experience
- Focusing on what is best not just for us as individuals, but for the overall
  community

Examples of unacceptable behavior include:

- The use of sexualized language or imagery, and sexual attention or advances
- Trolling, insulting or derogatory comments, and personal or political attacks
- Public or private harassment
- Publishing others' private information, such as a physical or email address,
  without their explicit permission
- Other conduct which could reasonably be considered inappropriate in a
  professional setting

## Enforcement Responsibilities

Community leaders are responsible for clarifying and enforcing our standards
of acceptable behavior and will take appropriate and fair corrective action in
response to any behavior that they deem inappropriate, threatening, offensive,
or harmful.

Community leaders have the right and responsibility to remove, edit, or reject
comments, commits, code, wiki edits, issues, and other contributions that are
not aligned to this Code of Conduct, and will communicate reasons for
moderation decisions when appropriate.

## Scope

This Code of Conduct applies within all community spaces, and also applies
when an individual is officially representing the community in public spaces.
Examples of representing our community include using an official e-mail
address, posting via an official social media account, or acting as an
appointed representative at an online or offline event.

## Enforcement

Instances of abusive, harassing, or otherwise unacceptable behavior may be
reported to the community leaders responsible for enforcement at
`<conduct@your-domain.example>`.

All complaints will be reviewed and investigated promptly and fairly.

All community leaders are obligated to respect the privacy and security of the
reporter of any incident.

## Enforcement Guidelines

Community leaders will follow these Community Impact Guidelines in determining
the consequences for any action they deem in violation of this Code of
Conduct:

### Correction

**Community Impact**: Use of inappropriate language or other behavior deemed
unprofessional or unwelcome in the community.

**Consequence**: A private, written warning from community leaders, providing
clarity around the nature of the violation and an explanation of why the
behavior was inappropriate. A public apology may be requested.

### Warning

**Community Impact**: A violation through a single incident or series of
actions.

**Consequence**: A warning with consequences for continued behavior. No
interaction with the people involved, including unsolicited interaction with
those enforcing the Code of Conduct, for a specified period of time. This
includes avoiding interactions in community spaces as well as external channels
like social media. Violating these terms may lead to a temporary or permanent
ban.

### Temporary Ban

**Community Impact**: A serious violation of community standards, including
sustained inappropriate behavior.

**Consequence**: A temporary ban from any sort of interaction or public
communication with the community for a specified period of time. No public or
private interaction with the people involved, including unsolicited
interaction with those enforcing the Code of Conduct, is allowed during this
period. Violating these terms may lead to a permanent ban.

### Permanent Ban

**Community Impact**: Demonstrating a pattern of violation of community
standards, including sustained inappropriate behavior, harassment of an
individual, or aggression toward or disparagement of classes of individuals.

**Consequence**: A permanent ban from any sort of public interaction within
the community.

## Attribution

This Code of Conduct is adapted from the Contributor Covenant, version 2.1,
available at https://www.contributor-covenant.org/version/2/1/code_of_conduct.html.

Community Impact Guidelines were inspired by
https://www.contributor-covenant.org/version/2/1/code_of_conduct.html#enforcement-guidelines.

---

## Purged root document: CONTRIBUTING.md

## Contributing to Aero

Thanks for helping build Aero.

Before contributing, please read:

- `LEGAL.md` (clean-room and distribution posture)
- `TRADEMARKS.md` (naming/branding guidance)
- `CODE_OF_CONDUCT.md`

### Licensing of contributions

Unless stated otherwise, contributions to this repository are accepted under
the project’s dual license: **MIT OR Apache-2.0** (see `LICENSE-MIT` and
`LICENSE-APACHE`).

Documentation under `docs/` is licensed under **CC BY 4.0** (see
`project-history.md`).

By submitting a pull request, you agree that your contribution may be
distributed under these terms.

### Clean-room / IP rules (critical)

This project must not incorporate Microsoft copyrighted material or become a
derivative work of GPL-only emulator code.

#### Do not copy from proprietary sources

Do **not** contribute code, tests, or docs derived from:

- Microsoft Windows source code (including leaked sources)
- Disassemblies/decompilations of Windows binaries
- Extracted Windows system files, drivers, fonts, icons, or UI assets
- Microsoft SDK headers/libraries that are not redistributable

#### Do not copy from GPL emulators

Do **not** copy code (or mechanically translated code) from GPL-licensed
projects such as QEMU or other GPL-only emulators/virtualizers.

You may use such projects as **high-level behavioral references** (e.g., “it
does X”) but not as copy/paste sources. When in doubt, rely on public specs or
black-box testing and write original code.

#### Prefer public specifications and citations

Good sources include:

- Intel/AMD Software Developer Manuals (SDM)
- PCI/PCIe, USB, SATA/AHCI, ACPI, VESA specifications
- W3C / WHATWG / WebGPU specifications for browser integration

When adding behavior based on a spec, include a link and section reference in
the PR description (and in docs when relevant).

### Prohibited files and artifacts

Do not commit or upload (including in issues/PRs):

- Windows ISOs, WIM/ESD files, update packages, or extracted file trees
- `.exe`, `.dll`, `.sys`, `.msi`, `.cab`, `.msu`, etc. from Windows
- Disk/VM images: `.vhd`, `.vhdx`, `.vmdk`, `.qcow2`, `.img`, `.raw`, etc.
  - Exception: a small number of **tiny, deterministic, license-safe** boot/test
    fixtures are allowlisted (see `../areas/testing.md`, e.g.
    `tests/fixtures/boot/*.{bin,img}`).
- ROM/firmware dumps from real hardware
- Captured traces/dumps that contain copyrighted payloads

`.gitignore` includes common patterns, but you are responsible even if Git
doesn’t catch something.

CI enforces these rules via `scripts/ci/check-repo-policy.sh`. See
`../areas/testing.md` for fixture alternatives (generate at runtime, download from
approved external sources, etc.).

If you need test programs, prefer:

- Small, self-authored test binaries whose source is included
- Existing permissively licensed test suites
- Synthetic test generators

### Source file headers (SPDX)

New source files should include an SPDX identifier:

- `SPDX-License-Identifier: MIT OR Apache-2.0` for code
- Documentation in `docs/` should follow `project-history.md` (CC BY 4.0)

### Tests and validation

When submitting a change:

- Run `cargo fmt --all` before pushing (CI enforces `cargo fmt --all -- --check`).
- Include unit tests where feasible.
- For compatibility behavior, include a minimal reproducible test (even if it
  only runs in the emulator).
- If you compare against Windows behavior, do it locally and describe the
  methodology in the PR. Do not upload Windows binaries/media as evidence.

### Pull requests and CI links

- Quick sanity suite: `npm run test:contracts`
- PR/CI links for your current branch:
  - `npm run pr:url` (compare/PR URL)
  - `npm run pr:links` (compare/PR URL + GitHub Actions URL)

### Dependency policy (licenses + advisories)

#### Automated updates (Dependabot)

This repository uses Dependabot to keep dependencies fresh with minimal maintainer
overhead. Update PRs are opened weekly and grouped to control noise:

- GitHub Actions: patch/minor updates grouped together; majors separated.
- npm: dev/tooling updates are grouped (Playwright + TS/Vite/Vitest toolchain).
- Rust (Cargo): patch/minor updates grouped; majors separated.
- Go modules: patch/minor updates grouped; majors separated.
- Terraform: updates grouped by provider (currently AWS).

#### Auto-merge policy (safe updates only)

Some Dependabot PRs are automatically approved and set to auto-merge **only**
after required CI checks pass:

- GitHub Actions: patch/minor updates.
- npm: patch/minor updates for an allowlisted set of tooling dependencies
  (Playwright + TS/Vite/Vitest + type packages).

By default, Dependabot PRs that touch runtime/production dependencies are **not**
auto-merged. Maintainers can explicitly opt a specific PR into auto-merge by
adding the `automerge-deps` label.

Auto-merge logic lives in `.github/workflows/dependabot-auto-merge.yml`. The
allowlist is intentionally narrow; widen it only with intent.

#### License allowlist (copyleft avoidance)

Aero’s IP posture explicitly forbids **GPL/LGPL/AGPL** (and similar copyleft)
contamination. CI enforces a strict **license allowlist** for third-party
dependencies across ecosystems.

Allowed licenses are expressed as SPDX identifiers (or SPDX expressions):

- `Apache-2.0`
- `MIT`
- `BSD-2-Clause`, `BSD-3-Clause`
- `ISC`
- `0BSD`
- `Zlib`
- `CC0-1.0`
- `CC-BY-3.0` (SPDX metadata packages in the npm ecosystem)
- `BSL-1.0`
- `Unicode-3.0`, `Unicode-DFS-2016`
- `BlueOak-1.0.0` (permissive; appears in the existing npm dependency graph)

How CI evaluates dependency licenses:

- **Dual-licensed** dependencies are allowed when their SPDX expression can be
  satisfied using allowlisted terms (e.g. `MIT OR Apache-2.0` is OK; `MIT OR
  GPL-3.0` is also OK because you can opt into MIT).
- **Conjunctive** expressions require every term to be allowlisted (e.g. `MIT AND
  Zlib` is OK; `MIT AND GPL-3.0` is not).
- **Unknown/missing** license metadata is treated as a CI failure. Fix it by
  switching dependencies or by ensuring upstream provides correct SPDX metadata
  (and re-run the check).
- **Vendored code** must include its license text in-tree and must use an
  allowlisted license. If you vendor code, include attribution + the license
  file(s) alongside the vendored directory.

#### License + vulnerability gating

CI enforces a dependency policy to help keep the project compatible with the
repository’s **MIT OR Apache-2.0** licensing and to catch known vulnerabilities
early:

- Rust: `cargo-deny` (`deny.toml`) checks license allowlist + banned sources
  (no git deps by default) + RustSec advisories.
- npm: `scripts/ci/check-npm-licenses.mjs` checks dependency licenses against
  the allowlist (fails on copyleft/unknown licenses). This runs on PRs so
  Dependabot auto-merge is gated by it.
- Go: `govulncheck` runs for `proxy/webrtc-udp-relay` when Go deps/code change,
  plus nightly/manual runs to catch newly published advisories.
- Go: `go-licenses` checks module dependency licenses for
  `proxy/webrtc-udp-relay` against the allowlist.
- npm: a scheduled `npm audit` runs against the lockfile for high/critical
  issues (nightly/manual only to avoid PR noise).

If you add or update dependencies and CI fails:

- Prefer switching to an equivalent permissively licensed crate.
- If a new license is acceptable for the project, update `deny.toml` with a
  justification in the PR.

### Security issues

Please do not open public issues for security vulnerabilities. See
`SECURITY.md` for reporting instructions.

---

### Purged root document: SECURITY.md

## Security Policy

### Reporting a vulnerability

Please **do not** open public issues for security vulnerabilities.

Preferred reporting channels:

1. GitHub Security Advisories (private report), if enabled for this repository.
2. If Security Advisories are not enabled, contact the maintainers via GitHub (without including sensitive details) to arrange a secure channel.

Include:

- A description of the vulnerability and impact
- Steps to reproduce (proof-of-concept if possible)
- Affected versions/commit hash (if known)
- Any suggested mitigation

### Response expectations

Maintainers will aim to:

- Acknowledge receipt within a reasonable time
- Work with the reporter to validate the issue
- Coordinate a fix and disclosure timeline

### Supported versions

Until the project has tagged releases, only the `main` branch is considered
supported for security fixes.

### Contributor security hygiene

For contributor-focused guidance (handling secrets, responding to leaks, and triaging CodeQL findings), see [`../areas/security.md`](../areas/security.md).

---

### Purged root document: LEGAL.md

## Legal & Compliance Summary

This document is **not legal advice**. It summarizes the project’s intent and
repo rules so development and distribution can proceed safely.

### What this project is (and is not)

**Aero is an emulator** (software that re-implements hardware behavior). Aero
does **not** include Microsoft Windows, Microsoft drivers, Microsoft firmware,
or any other Microsoft copyrighted code.

### Windows media, keys, and licensing (users supply their own)

If you run Windows inside Aero:

- You must supply your **own legally obtained Windows installation media**
  (e.g., ISO) and any required product keys.
- You are responsible for complying with the applicable Windows EULA and any
  other third-party license terms for software you run.
- Aero must **not** bypass Windows activation or DRM. Aero aims to provide
  compatibility, not circumvention.

The project **does not**:

- Distribute Windows images, ISOs, WIM/ESD files, updates, drivers, or other
  Microsoft components.
- Provide links to pirated content, activation cracks, or instructions to
  circumvent protections.

### Prohibited content in this repository

Do not commit or attach (including in issues/PRs) any of the following:

- Windows installation media or system files (ISO/WIM/ESD/CAB/MSU/MSI, etc.)
- Microsoft binaries (e.g., `.exe`, `.dll`, `.sys`) or extracted file trees
- ROM/firmware dumps from real hardware
- Licensed or proprietary SDKs/tools that cannot be redistributed
- Captured traces or dumps that include copyrighted payloads

See `.gitignore` and `CONTRIBUTING.md` for examples and enforcement guidance.
CI also enforces these rules via `scripts/ci/check-repo-policy.sh` (see
`../areas/testing.md`).

### Clean-room expectations

Aero is intended to be implemented from:

- Public specifications and standards
- Clean-room reverse engineering (black-box observation)
- Original work authored by contributors

Contributors **must not** copy code from:

- Microsoft Windows source/binaries or leaked materials
- GPL-licensed emulators/virtualizers in a way that creates a derivative work
  (e.g., QEMU) unless the licensing implications are understood and accepted
  by the project (current intent: **avoid GPL-only code paths**)

See `CONTRIBUTING.md` for concrete rules on citations, testing, and acceptable
reference material.

### Trademarks / no affiliation

Microsoft, Windows, Windows 7, and related marks are trademarks of Microsoft
Corporation. Aero is an independent project and is **not affiliated with,
endorsed by, or sponsored by** Microsoft.

See `TRADEMARKS.md` for naming and branding guidelines (including allowed
nominative use like “runs Windows 7” vs. confusing branding).

### DMCA posture (takedowns & repeat infringement)

If you believe content in this repository infringes your rights, see
`DMCA_POLICY.md` for the takedown/counter-notice process.

The project’s intent is to:

- Remove infringing material when properly notified
- Keep an auditable record of actions taken
- Discourage repeat infringement

### Licensing overview

- **Source code** (and most repository content) is dual-licensed:
  **MIT OR Apache-2.0** (see `LICENSE-MIT` and `LICENSE-APACHE`).
- **Documentation in `docs/`** is licensed under **CC BY 4.0**
  (see `project-history.md`).

For attribution guidance, see `NOTICE` and `AUTHORS`.

### Hosted service templates

If Aero is ever offered as a hosted demo/service, start from:

- `TERMS_OF_SERVICE_TEMPLATE.md`
- `PRIVACY_POLICY_TEMPLATE.md`

These templates include the expected “users provide their own Windows media”
and “no circumvention / no infringement” posture.

---

### Purged root document: TRADEMARKS.md

## Trademark & Branding Guidelines

This document is not legal advice. Its goal is to reduce confusion and avoid
implying affiliation with Microsoft or other trademark owners.

### Microsoft trademarks

Microsoft, Windows, Windows 7, and related marks are trademarks of Microsoft
Corporation. Aero is not affiliated with, endorsed by, or sponsored by
Microsoft.

### Allowed (generally “nominative” use)

You may use Microsoft marks **only as needed to truthfully describe
compatibility**, and only in a way that does not suggest endorsement.

Examples that are typically acceptable:

- “Runs Windows 7 (user-supplied media required)”
- “Windows 7 compatible emulator”
- “Tested with Windows 7 SP1 x86”

Best practices:

- Use trademarks as adjectives (“Windows 7 system”), not as nouns (“a Windows”)
- Include a clear **no-affiliation** disclaimer near any prominent use
- Avoid using Microsoft product icons/logos/artwork

### Forbidden / high-risk uses

Do not:

- Use Microsoft logos, product icons, or branding assets
- Name the project or a fork in a way that implies it *is* Windows or is
  officially produced by Microsoft (e.g., “Windows Emulator”, “Win7Emu”)
- Register domains or social accounts that look official or confusingly
  similar to Microsoft properties
- Use “Windows” as the primary brand name of your distribution

### Project naming guidance

Preferred naming patterns:

- “Aero” (project name) with a subtitle like “an x86 PC emulator”
- “Aero Emulator” / “Aero Web Emulator”

Avoid naming patterns that lead with Microsoft marks:

- “Windows 7 in the Browser”
- “Microsoft Windows Emulator”

### UI and marketing copy

Suggested disclaimer text:

> Aero is an independent project and is not affiliated with, endorsed by, or
> sponsored by Microsoft Corporation. Windows is a trademark of Microsoft
> Corporation.

If you distribute a hosted service, also include:

> Users must supply their own Windows installation media and valid licenses.
> Aero does not provide or distribute Microsoft Windows.

---

### Purged root document: DMCA_POLICY.md

## DMCA Takedown Policy (Template)

This repository aims to contain only original, clean-room implementation code
and freely licensed documentation. It must **not** host Microsoft Windows
media/binaries or other infringing material.

This policy is a template for maintainers. It is not legal advice.

### Designated contact

Send takedown notices to:

- Email: `<dmca@your-domain.example>`
- Subject: `DMCA Takedown Notice - Aero`

If you cannot email, open a GitHub issue **only if** it does not require
posting copyrighted material publicly.

### Takedown notice requirements

To help us act quickly, include:

1. Your name and contact information (email + mailing address).
2. Identification of the copyrighted work claimed to have been infringed.
3. Identification of the allegedly infringing material, with enough
   information for us to locate it (URL(s), commit hash, file path, etc.).
4. A statement that you have a good faith belief that the use is not
   authorized by the copyright owner, its agent, or the law.
5. A statement, under penalty of perjury, that the information in the notice
   is accurate and that you are the copyright owner or authorized to act on
   the owner’s behalf.
6. A physical or electronic signature.

### What maintainers will do

Upon receiving a facially valid notice, maintainers will:

1. Acknowledge receipt.
2. Remove or disable access to the identified material (as appropriate).
3. Document the action taken (e.g., in a private maintainer log).
4. Notify the contributor/uploader (if applicable) with a copy of the notice
   (redacting personal information where appropriate).

### Counter-notice process

If you believe your content was removed in error, you may submit a
counter-notice containing:

1. Your name, address, and phone number.
2. Identification of the removed material and its location before removal.
3. A statement, under penalty of perjury, that you have a good faith belief
   the material was removed as a result of mistake or misidentification.
4. A statement consenting to the jurisdiction of the appropriate federal
   district court (for the relevant hosting provider) and that you will accept
   service of process from the complainant.
5. A physical or electronic signature.

Maintainership may restore the material if appropriate and permitted by the
hosting provider’s policies.

### Repeat infringer policy

Accounts that repeatedly upload infringing material (including Windows ISOs,
WIMs, extracted system files, or other proprietary payloads) may be blocked
from contributing.

---

### Purged root document: PRIVACY_POLICY_TEMPLATE.md

## Privacy Policy (Template)

> Replace bracketed placeholders (e.g., `<SERVICE_NAME>`) before publishing.
> This template is not legal advice.

**Effective date:** `<YYYY-MM-DD>`

This Privacy Policy describes how `<SERVICE_NAME>` (“we”, “us”) collects, uses,
and shares information when you access the service at `<SERVICE_URL>`.

### Summary

- We aim to run the emulator locally in your browser.
- Users must provide their own Windows installation media and licenses.
- This policy must accurately describe whether any files (disk images, ISOs,
  logs, crash reports) are uploaded to servers.

### Information we collect

#### Information you provide

- **Account information** (if accounts exist): email address, username, etc.
- **Support requests**: information you include when contacting us.
- **User-provided files** (only if the service uploads them): disk images,
  ISOs, or other files you choose to provide.

#### Information collected automatically

- **Usage data**: pages viewed, feature usage, timestamps.
- **Device/browser data**: user agent, OS, approximate locale.
- **Diagnostics**: crash reports and performance metrics (if enabled).
- **IP address**: typically collected by servers as part of normal operation.

#### Cookies / local storage

Describe any cookies or local storage used for:

- Session management
- Preferences
- Analytics

If the emulator stores state locally (e.g., OPFS/IndexedDB), describe what is
stored and how to clear it.

### How we use information

We use information to:

- Provide and maintain the service
- Improve performance and reliability
- Respond to support requests
- Prevent abuse and enforce terms

### Sharing

We do not sell personal information.

We may share information with:

- Service providers (hosting, analytics) acting on our behalf
- Law enforcement when required by law

If user-provided files are uploaded, describe:

- Where they are stored
- Who can access them
- Whether they are encrypted at rest/in transit

### Data retention

Describe retention periods for:

- Account data
- Server logs
- Uploaded files (if any)

### Security

We use reasonable technical and organizational measures to protect data.
However, no method of transmission or storage is 100% secure.

### Your choices

Describe how users can:

- Access, update, or delete account data
- Opt out of analytics (if applicable)
- Delete local emulator state (e.g., clear site data)

### International users

If applicable, describe international transfers and legal bases.

### Changes

We may update this policy. We will post the updated version with a new
effective date.

### Contact

Privacy questions: `<privacy@your-domain.example>`

---

### Purged root document: TERMS_OF_SERVICE_TEMPLATE.md

## Terms of Service (Template)

> Replace bracketed placeholders (e.g., `<SERVICE_NAME>`) before publishing.
> This template is not legal advice.

**Effective date:** `<YYYY-MM-DD>`

These Terms of Service (“Terms”) govern your access to `<SERVICE_NAME>` at
`<SERVICE_URL>` (the “Service”).

### Acceptance

By accessing or using the Service, you agree to these Terms.

### The Service

The Service provides access to the Aero emulator and related tooling.

The Service **does not provide Microsoft Windows** or other proprietary
software. Users are responsible for supplying any operating system media and
licenses required to use the emulator.

### Eligibility

You must comply with applicable laws and be able to form a binding agreement
in your jurisdiction.

### User-provided content (uploads)

If the Service allows uploading files (e.g., disk images or installation media):

- You represent that you have the necessary rights to upload and use the
  content, including compliance with any applicable licenses/EULAs.
- You must not upload content that infringes intellectual property rights or
  violates law.
- You acknowledge that uploading may create copies on our servers and in
  transit; do not upload content unless you are authorized to do so.

We may remove content at our discretion, including in response to takedown
requests. See `DMCA_POLICY.md` for the project’s posture.

### Prohibited conduct

You agree not to:

- Use the Service to distribute pirated software or infringing material
- Attempt to bypass activation/DRM or provide circumvention tools
- Interfere with the Service (e.g., abuse, probing, excessive load)
- Misrepresent affiliation with Microsoft or other parties

### Intellectual property

Aero’s source code is dual-licensed under **MIT OR Apache-2.0** (see
`LICENSE-MIT` and `LICENSE-APACHE`). Documentation in `docs/` is licensed under
**CC BY 4.0** (see `project-history.md`).

These Terms do not grant any rights to third-party software you run inside the
emulator.

### Trademarks / no affiliation

Microsoft, Windows, and related marks are trademarks of Microsoft Corporation.
The Service is not affiliated with, endorsed by, or sponsored by Microsoft.

See `TRADEMARKS.md` for guidance.

### Disclaimer

THE SERVICE IS PROVIDED “AS IS” AND “AS AVAILABLE” WITHOUT WARRANTIES OF ANY
KIND, WHETHER EXPRESS OR IMPLIED, INCLUDING MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE, AND NON-INFRINGEMENT.

### Limitation of liability

TO THE MAXIMUM EXTENT PERMITTED BY LAW, IN NO EVENT WILL THE SERVICE OPERATOR
BE LIABLE FOR INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL, OR PUNITIVE
DAMAGES, OR ANY LOSS OF PROFITS OR DATA, ARISING OUT OF OR RELATED TO YOUR USE
OF THE SERVICE.

### Termination

We may suspend or terminate access if you violate these Terms.

### Changes

We may update these Terms. We will post the updated version with a new
effective date.

### Contact

Questions about these Terms: `<terms@your-domain.example>`

---

### Purged tree document: server/LEGACY.md

## Legacy: Aero backend server (TCP proxy + COOP/COEP static hosting)

This document describes the **legacy** `server/` backend. New work should target the Aero Gateway contract instead:

- `services/gateway`
- [`../specs/gateway-api.md`](../specs/gateway-api.md)
- [`services/gateway/openapi.yaml`](../../services/gateway/openapi.yaml)

---

This package provides a small backend service required by early Aero prototypes:

1. **Static file hosting** with **COOP/COEP** headers (Cross-Origin Isolation for `SharedArrayBuffer`).
2. A **secure WebSocket TCP proxy** (`/ws/tcp`) so the browser can open outbound TCP connections.
3. A small **DNS lookup HTTP API** (`/api/dns/lookup`) for cases where the client is not using DoH directly.

### Quick start (local dev)

```bash
# From the repo root (npm workspaces)
npm ci

# Required. Pick any random string in real deployments.
export AERO_PROXY_TOKEN="dev-token"

# For local testing you must explicitly allow targets.
# This example allows connecting to any *public* host on 80/443.
export AERO_PROXY_ALLOW_HOSTS="*"
export AERO_PROXY_ALLOW_PORTS="80,443"

npm -w server run dev
```

Open: `http://localhost:8080/`

### Security model

This server is designed to **not** be an open proxy by default:

- **Authentication token is required** for WebSocket proxying and DNS API calls.
- **Outbound targets are deny-by-default** via `AERO_PROXY_ALLOW_HOSTS` and `AERO_PROXY_ALLOW_PORTS`.
- **Private address ranges are blocked** unless `AERO_PROXY_ALLOW_PRIVATE_RANGES=1`.
- Per-client **connection limits** and basic **bandwidth caps** are enforced.

If you loosen the allowlist (e.g. `AERO_PROXY_ALLOW_HOSTS="*"`), the token becomes the primary line of defense.
Treat it like a password.

### COOP/COEP

For cross-origin isolation, all HTTP responses include:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp`
- `Cross-Origin-Resource-Policy: same-origin`
- `Origin-Agent-Cluster: ?1`

If you serve the frontend behind a reverse proxy (nginx, Caddy, etc), ensure those headers are preserved.

This legacy server also sets a strict Content Security Policy (CSP) compatible with Aero’s requirements (including dynamic WASM compilation for JIT via `script-src 'wasm-unsafe-eval'`).
See [`../areas/security.md`](../areas/security.md) for rationale and tradeoffs.

#### nginx snippet

```nginx
add_header Cross-Origin-Opener-Policy "same-origin" always;
add_header Cross-Origin-Embedder-Policy "require-corp" always;
add_header Cross-Origin-Resource-Policy "same-origin" always;
add_header Origin-Agent-Cluster "?1" always;
```

### Endpoints

#### Static hosting

- `GET /` serves `server/public/index.html` by default.

Set a custom directory with:
`AERO_PROXY_STATIC_DIR=/path/to/frontend/dist`.

#### WebSocket TCP proxy

- `WS /ws/tcp?token=...`

Binary protocol (big-endian):

Client → server:

- `0x01 CONNECT`
  - `u8  type = 0x01`
  - `u32 connId`
  - `u8  addrType` (`0x01=hostname`, `0x02=ipv4`, `0x03=ipv6`)
  - `u8  addrLen`
  - `u8[addrLen] addr`
  - `u16 port`
- `0x02 DATA`
  - `u8 type = 0x02`
  - `u32 connId`
  - `bytes payload`
- `0x03 END`
  - `u8 type = 0x03`
  - `u32 connId`
- `0x04 CLOSE`
  - `u8 type = 0x04`
  - `u32 connId`

Server → client:

- `0x10 OPENED`
  - `u8  type = 0x10`
  - `u32 connId`
  - `u8  status` (`0=ok`, non-zero=failed)
  - `u16 msgLen`
  - `u8[msgLen] msg (utf8)`
- `0x11 DATA`
  - `u8 type = 0x11`
  - `u32 connId`
  - `bytes payload`
- `0x12 END`
  - `u8 type = 0x12`
  - `u32 connId`
- `0x13 CLOSE`
  - `u8  type = 0x13`
  - `u32 connId`
  - `u8  reason`
  - `u16 msgLen`
  - `u8[msgLen] msg (utf8)`

#### DNS lookup

- `GET /api/dns/lookup?name=example.com&token=...`

Response:

```json
{ "name": "example.com", "addresses": [ { "address": "93.184.216.34", "family": 4 } ] }
```

### Configuration

| Env var | Description | Default |
| --- | --- | --- |
| `AERO_PROXY_HOST` | Bind address | `0.0.0.0` |
| `AERO_PROXY_PORT` | HTTP port | `8080` |
| `AERO_PROXY_TOKEN` | Required auth token | (required) |
| `AERO_PROXY_ALLOW_HOSTS` | Comma-separated allowlist (`*`, `example.com`, `*.example.com`, `1.2.3.4`, `10.0.0.0/8`) | *(deny all)* |
| `AERO_PROXY_ALLOW_PORTS` | Comma-separated ports/ranges (`80,443,10000-10100` or `*`) | *(deny all)* |
| `AERO_PROXY_ALLOW_PRIVATE_RANGES` | Allow private/loopback/link-local ranges (`1`/`0`) | `0` |
| `AERO_PROXY_MAX_TCP_PER_WS` | TCP conns per WebSocket | `8` |
| `AERO_PROXY_MAX_TCP_TOTAL` | TCP conns across all clients | `512` |
| `AERO_PROXY_MAX_WS_PER_IP` | WebSocket conns per remote IP | `4` |
| `AERO_PROXY_BANDWIDTH_BPS` | Per-direction bytes/sec cap per WebSocket | `5000000` |
| `AERO_PROXY_CONNECTS_PER_MINUTE` | TCP CONNECT frames per minute per WebSocket | `60` |
| `AERO_PROXY_MAX_WS_MESSAGE_BYTES` | Max WebSocket message size | `1048576` |
| `AERO_PROXY_ALLOWED_ORIGINS` | Optional comma-separated Origin allowlist for WS/DNS | *(disabled)* |

---

### Static file server with Range + CORS (for streaming disk images)

For local development/testing of the streaming disk backend, this repo also
includes a tiny standalone server script:

```bash
node server/range_server.js --dir /path/to/images --port 8081
```

It serves files with:

- HTTP `Range` support (`206 Partial Content`)
- CORS headers suitable for browser Range reads
- Optional COOP/COEP headers (`--coop-coep`)

This is intended for development only; it is not hardened.

---

### Purged tree document: server/README.md

## Aero backend server (legacy)

This `server/` package was an earlier monolithic backend for Aero (static hosting + TCP proxy + DNS lookup).

**It is being superseded by the Aero Gateway backend contract**:

- Implementation: `services/gateway`
- API contract: [`../specs/gateway-api.md`](../specs/gateway-api.md)
- OpenAPI (HTTP endpoints): [`services/gateway/openapi.yaml`](../../services/gateway/openapi.yaml)

For browser-side requirements (COOP/COEP / cross-origin isolation), see:

- [`../areas/web-host.md`](../areas/web-host.md#cross-origin-isolation-coopcoep-deployment-requirements)
- [`../areas/build-and-tooling.md`](../areas/build-and-tooling.md)
- [`../areas/security.md`](../areas/security.md)

### Legacy protocol docs

If you need the old endpoints/protocols for historical prototypes, see [`server/LEGACY.md`](./purged-material.md).

### Dev helpers (disk streaming)

This directory also contains standalone dev-only helpers used by the disk streaming conformance tooling:

- `server/range_server.js`: static file server with HTTP Range + CORS headers
  - Used by `tools/disk-streaming-conformance/selftest_range_server.py`
  - Supports `--auth-token <Authorization-value>` for private-mode selftests
- `server/chunk_server.js`: static server for chunked disk images (`manifest.json` + `chunks/*.bin`)
  - Used by `tools/disk-streaming-conformance/selftest_chunk_server.py`
  - Supports `--auth-token <Authorization-value>` for private-mode selftests

---

### Purged tree document: server/disk-gateway/README.md

## disk-gateway (reference)

An authenticated, HTTP Range-capable disk-image gateway intended for the browser `StreamingDisk` design.

This server is a **reference implementation** for deployments that need:

- Efficient random-access reads via `Range: bytes=start-end` (single + multipart multi-range).
- Public and private images on local filesystem.
- Short-lived signed “disk access lease” tokens (JWT HS256).
- Correct CORS preflights (including `Range` + `Authorization`).
- `Cross-Origin-Resource-Policy` headers for COEP/CORP compatibility.

### Running locally

```bash
cd server/disk-gateway

export DISK_GATEWAY_BIND=127.0.0.1:3000
export DISK_GATEWAY_PUBLIC_DIR=./public-images
export DISK_GATEWAY_PRIVATE_DIR=./private-images
export DISK_GATEWAY_TOKEN_SECRET='dev-secret-change-me'

# CORS allowlist (comma-separated). Use "*" to allow any Origin (no credentials).
export DISK_GATEWAY_CORS_ALLOWED_ORIGINS='http://localhost:5173'

# CORP policy for /disk/* responses: "same-site" (default) or "cross-origin"
export DISK_GATEWAY_CORP='same-site'

# Multi-range abuse guards (defaults shown).
export DISK_GATEWAY_MAX_RANGES=16
export DISK_GATEWAY_MAX_TOTAL_BYTES=$((512 * 1024 * 1024))  # 512 MiB

cargo run --locked
```

#### File layout (dev filesystem backend)

This reference server uses a simple naming convention:

- **Public** image id `win7` → `${DISK_GATEWAY_PUBLIC_DIR}/win7.img`
- **Private** image id `secret` for user `alice` → `${DISK_GATEWAY_PRIVATE_DIR}/alice/secret.img`

Both `{id}` and `{user}` are validated as “path segments” (letters/digits/`._-`, excluding `.` and `..`) to avoid path traversal.

### API

#### `POST /api/images/{id}/lease`

Issues a short-lived signed “lease” token for private images.

- Public images: returns a `url` and no token.
- Private images: **requires a caller identity** via one of:
  - `X-Debug-User: <user-id>` (placeholder auth)
  - `Authorization: Bearer <user-id>` (placeholder auth)
  and returns a JWT.

Response:

```json
{ "url": "/disk/<id>", "token": "<jwt>", "expiresAt": "2026-01-01T00:00:00Z" }
```

Warning: `X-Debug-User` is **not production auth**. Replace this with real authentication/authorization before deploying.

#### `GET /disk/{id}` (bytes)

- Supports `Range: bytes=start-end` and multipart multi-range (`bytes=0-0,2-2`).
- Returns `206 Partial Content` with correct `Content-Range` when Range is present.
- Returns `416 Range Not Satisfiable` with `Content-Range: bytes */<size>` for invalid/unsatisfiable ranges.
- Returns `413 Payload Too Large` when multi-range limits are exceeded (`DISK_GATEWAY_MAX_RANGES`, `DISK_GATEWAY_MAX_TOTAL_BYTES`).
- Sets `Accept-Ranges: bytes`, `Content-Length`, and a (strong) `ETag`.
- Sets `Last-Modified` when filesystem metadata is available (omitted for pre-epoch mtimes).
- Sets `Content-Type: application/octet-stream` and `X-Content-Type-Options: nosniff`.
- Sets `Content-Encoding: identity` to make “no compression transforms” explicit.
- Supports basic conditional requests:
  - `If-None-Match` → `304 Not Modified`
  - `If-Modified-Since` → `304 Not Modified` (when `If-None-Match` is absent)
  - `If-Range` + `Range` → `206` when matched, otherwise ignores `Range` and returns a full `200`
    - Both entity-tag and HTTP-date forms are supported.
- Sets `Cache-Control: no-transform` to prevent intermediaries from applying compression to raw disk bytes.
  - For authenticated requests (private images), also sets `Cache-Control: private, no-store, no-transform`.

Private images require a valid lease token:

- Preferred: `Authorization: Bearer <token>`
- Optional: `?token=<token>`

Security note: Query-string tokens can leak via logs, caches, and `Referer` headers. Prefer `Authorization`.

#### `HEAD /disk/{id}`

Returns headers (e.g. `Content-Length`, `Accept-Ranges`, `ETag`) without a body.

#### CORS preflight

- `OPTIONS /disk/{id}` and `OPTIONS /api/*` return `204` with:
  - `Access-Control-Allow-Methods`
  - `Access-Control-Allow-Headers` including `Range, If-Range, If-None-Match, If-Modified-Since, Authorization, Content-Type`
  - `Access-Control-Allow-Origin` from `DISK_GATEWAY_CORS_ALLOWED_ORIGINS`
  - `Access-Control-Max-Age: 86400`

### Examples

#### Range read (public)

```bash
curl -v -H 'Range: bytes=0-15' \
  http://127.0.0.1:3000/disk/win7 \
  -o /tmp/first-16-bytes.bin
```

#### Lease + Range read (private)

```bash
TOKEN="$(curl -s -X POST \
  -H 'X-Debug-User: alice' \
  http://127.0.0.1:3000/api/images/secret/lease \
  | jq -r .token)"

curl -v -H "Authorization: Bearer $TOKEN" \
  -H 'Range: bytes=0-1023' \
  http://127.0.0.1:3000/disk/secret \
  -o /tmp/first-1k.bin
```

#### Browser fetch snippet (Range)

```js
const token = "<jwt>";
const res = await fetch("https://disk.example.com/disk/secret", {
  headers: {
    "Range": "bytes=0-1048575",
    "Authorization": `Bearer ${token}`,
  },
});
if (res.status !== 206) throw new Error(`Expected 206, got ${res.status}`);
const chunk = new Uint8Array(await res.arrayBuffer());
```

#### CORS preflight smoke test

```bash
curl -i -X OPTIONS \
  -H 'Origin: https://app.example' \
  -H 'Access-Control-Request-Method: GET' \
  -H 'Access-Control-Request-Headers: Range, Authorization' \
  http://127.0.0.1:3000/disk/win7
```

### COEP/CORP + CORS header matrix

This server always sets CORS headers on `/disk/*` responses (when the request `Origin` is allowed) and sets
`Cross-Origin-Resource-Policy` based on `DISK_GATEWAY_CORP`.

| Deployment relationship (app → disk host) | Suggested `DISK_GATEWAY_CORP` | CORS (`DISK_GATEWAY_CORS_ALLOWED_ORIGINS`) |
| --- | --- | --- |
| Same-origin | `same-site` | Not required (but harmless) |
| Same-site (e.g. `app.example.com` → `disk.example.com`) | `same-site` | Allow the app origin |
| Cross-site | `cross-origin` | Allow the app origin (or `*` for non-credentialed requests) |

For a cross-origin isolated app (`Cross-Origin-Embedder-Policy: require-corp`), the disk resource must be
either CORS-enabled and/or explicitly allow embedding via CORP. Configure both correctly for your topology.

---

### Purged tree document: services/image-gateway/README.md

## image-gateway (reference implementation)

`image-gateway` is a small backend service that enables **secure browser streaming of large disk images** stored in S3 (or S3-compatible) via **HTTP Range**.

Related docs:
- [Disk Image Lifecycle and Access Control](../areas/storage.md) (hosted uploads/ownership/sharing/writeback)
- [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md) (normative disk-bytes endpoint behavior)
- [Chunked Disk Image Format (no Range)](../specs/chunked-disk-image-format.md) (manifest + chunk objects; avoids CORS preflight)
- [deployment/cloudfront-disk-streaming.md](../areas/storage.md) (concrete CloudFront setup)

The intended production path is:

1. Client uploads the disk image to S3 using **multipart upload** with presigned `PUT` URLs.
2. Client requests a **stable** CloudFront URL for the image (no query-string signing) plus **CloudFront signed-cookie auth material**.
3. Browser streams the image directly from CloudFront using `Range` requests (no proxying all bytes through the app).

This service implements a minimal, swappable auth + owner model (dev stub) and stores image records **in memory** (reference only).

API documentation: see ``openapi.yaml``.

### Disk object headers (CloudFront/S3 fast path)

In production, the high-throughput path is **CloudFront → S3** (the client streams bytes directly from the CDN).
That means response headers do **not** come from `GET /v1/images/:id/range` (the proxy fallback); they come from the
S3 object metadata that was set when the object was created.

When starting the multipart upload, `image-gateway` sets these S3 object headers:

- `Content-Type: application/octet-stream`
- `Cache-Control: …, no-transform` (see `IMAGE_CACHE_CONTROL` below)
- `Content-Encoding: identity`

These are required defence-in-depth to prevent CDNs/intermediaries from applying transforms (especially compression)
that would make disk `Range` offsets meaningless.

This only affects **newly created** objects; existing S3 objects will keep whatever metadata they currently have.

#### CloudFront “Compress objects automatically”

For disk images, CloudFront compression must be **disabled**. If “Compress objects automatically” is enabled for the
disk cache behavior, CloudFront may return a `Content-Encoding` other than `identity`, breaking deterministic ranges.

### Setup

```bash
# From the repo root (npm workspaces)
npm ci
```

Copy `.env.example` to `.env` and fill in values:

```bash
cp .env.example .env
```

#### Environment variables

Required:

- `S3_BUCKET`
- `AWS_REGION`

Credentials:

The AWS SDK uses the standard default credential provider chain (env vars, shared config files, instance roles, etc).
For local MinIO, set:

- `AWS_ACCESS_KEY_ID`
- `AWS_SECRET_ACCESS_KEY`

Optional (S3-compatible / MinIO):

- `S3_ENDPOINT` (e.g. `http://127.0.0.1:9000`)
- `S3_FORCE_PATH_STYLE=true|false` (often `true` for MinIO)

CloudFront (required for `/v1/images/:id/stream-url` in `cookie`/`url` mode):

- `CLOUDFRONT_DOMAIN` (e.g. `dxxxxx.cloudfront.net` or `images.example.com`)
- `CLOUDFRONT_KEY_PAIR_ID`
- `CLOUDFRONT_PRIVATE_KEY_PEM` (either the PEM string **or** a filesystem path to a PEM file)

Service:

- `IMAGE_BASE_PATH` (default `/images`)
- `AUTH_MODE=dev|none` (default `dev`)
- `PORT` (default `3000`)

Useful optional knobs:

- `CLOUDFRONT_AUTH_MODE=cookie|url` (default `cookie`)
- `CLOUDFRONT_COOKIE_DOMAIN` (optional; e.g. `.example.com` when API runs on `api.example.com` and CloudFront on `images.example.com`)
- `CLOUDFRONT_COOKIE_SAMESITE=None|Lax|Strict` (default `None`; use `Lax`/`Strict` when streaming is same-site and you don't need third-party cookies)
- `CLOUDFRONT_COOKIE_PARTITIONED=true|false` (default `false`; adds the `Partitioned` attribute for CHIPS-capable browsers, requires `CLOUDFRONT_COOKIE_SAMESITE=None`)
- `CORS_ALLOW_ORIGIN` (default `*`, used for browser CORS; set an explicit origin if you need credentialed requests / cookie-based auth)
- `CROSS_ORIGIN_RESOURCE_POLICY` (default `same-site`, sent on the range-proxy responses as defence-in-depth for COEP; see `../areas/storage.md`)
- `MULTIPART_PART_SIZE_BYTES` (default `67108864` / 64MiB; must be 5MiB–5GiB)
- `IMAGE_CACHE_CONTROL=private-no-store|public-immutable` (default `private-no-store`)
  - `private-no-store` sets `Cache-Control: private, no-store, no-transform` (safe for private images)
  - `public-immutable` sets `Cache-Control: public, max-age=31536000, immutable, no-transform` (only safe when keys are immutable/versioned and access control is enforced elsewhere, e.g. signed CloudFront URL/cookie)

#### Local MinIO (optional)

For local development without AWS, you can run MinIO:

```bash
docker compose -f docker-compose.minio.yml up
```

Alternatively, if you just want a general-purpose local S3-compatible object store for Range + CORS testing,
the repo also provides [`infra/local-object-store/`](../decisions/README.md).

That setup includes an optional nginx “edge” proxy and a `verify.sh` smoke test. If you use it with `image-gateway`,
set `BUCKET_NAME` to match your `S3_BUCKET` (defaults differ), and note it configures the bucket for anonymous download
by default to simplify browser/curl testing.

Then set:

- `S3_ENDPOINT=http://127.0.0.1:9000`
- `S3_FORCE_PATH_STYLE=true`
- `S3_BUCKET=aero-images` (or change the bucket name in `.env` and in `docker-compose.minio.yml`)
- `AWS_ACCESS_KEY_ID=minioadmin`
- `AWS_SECRET_ACCESS_KEY=minioadmin`

#### Run

```bash
npm -w services/image-gateway run dev
```

Health endpoints:

- `GET /health` / `GET /healthz` (liveness)
- `GET /readyz` (checks S3 bucket reachability)

### Multipart upload flow (curl)

These are illustrative; real clients should upload parts from the browser using `File.slice()` and `fetch()` / `PUT`.

Assuming `AUTH_MODE=dev`:

```bash
export USER_ID=dev-user
export API=http://localhost:3000
```

#### Create an image + start multipart upload

```bash
curl -sS -X POST "$API/v1/images" \
  -H "X-User-Id: $USER_ID" | jq
```

Response:

```json
{ "imageId": "...", "uploadId": "...", "partSize": 67108864 }
```

#### Request an upload URL for a part

```bash
curl -sS -X POST "$API/v1/images/<imageId>/upload-url" \
  -H "X-User-Id: $USER_ID" \
  -H "content-type: application/json" \
  -d '{"uploadId":"<uploadId>","partNumber":1}' | jq -r .url
```

#### Upload the part to S3

```bash
UPLOAD_URL="$(curl -sS -X POST "$API/v1/images/<imageId>/upload-url" \
  -H "X-User-Id: $USER_ID" \
  -H "content-type: application/json" \
  -d '{"uploadId":"<uploadId>","partNumber":1}' | jq -r .url)"

# Example: upload first 64MiB from disk.img
dd if=disk.img of=part1.bin bs=1m count=64

curl -i -X PUT --upload-file part1.bin "$UPLOAD_URL"
# Capture the `ETag` response header for completion.
```

#### Complete multipart upload

```bash
curl -sS -X POST "$API/v1/images/<imageId>/complete" \
  -H "X-User-Id: $USER_ID" \
  -H "content-type: application/json" \
  -d '{"uploadId":"<uploadId>","parts":[{"partNumber":1,"etag":"\"<etag-from-put>\""}]}' | jq
```

### Getting a stream URL (CloudFront)

```bash
curl -i -sS "$API/v1/images/<imageId>/stream-url" \
  -H "X-User-Id: $USER_ID"
```

If `CLOUDFRONT_AUTH_MODE=cookie`, the response includes:

- a stable `url` (no query string)
- `Set-Cookie` headers (CloudFront signed cookies)
- the same cookie strings in JSON under `auth.cookies`

The browser can then issue `Range` requests to `url` and CloudFront will authorize using the signed cookies.

If your API host cannot set cookies for the CloudFront domain (common when using the default `*.cloudfront.net` domain),
set `CLOUDFRONT_AUTH_MODE=url` to return a signed URL instead.

### Range proxy fallback (local dev)

`GET /v1/images/:imageId/range` streams bytes from S3 using `GetObject` + `Range`.

This is useful when you don't have CloudFront locally, but it proxies all bytes through the app (not recommended for production).

The service includes CORS headers and supports `OPTIONS` preflight for browser use. Set `CORS_ALLOW_ORIGIN` as needed.

`HEAD /v1/images/:imageId/range` is also supported for size discovery (mirrors what `StreamingDisk` does against the CloudFront URL).

Note: only **single-range** requests are supported (e.g. `Range: bytes=a-b` or `Range: bytes=a-`).
Malformed/multi-range requests (e.g. `Range: bytes=a-b,c-d`) return `416 Range Not Satisfiable`.

### Notes for browser `StreamingDisk`

The browser side should:

1. Call `GET /v1/images/:imageId/stream-url` once to obtain `url` and apply `auth` (cookies or signed URL).
2. Use `fetch(url, { headers: { Range: "bytes=start-end" } })` for chunk reads.
3. Prefer `ETag` + `Content-Range` to validate ranges.

CloudFront must be configured to:

- allow `Range` requests (forward the `Range` header to S3)
- return `206 Partial Content` responses for ranged reads

### Chunked disk image delivery (no `Range`)

In addition to `Range` streaming, `image-gateway` can serve disk images in a **chunked** format:

- `manifest.json` + `chunks/00000000.bin`, `chunks/00000001.bin`, ...
- Plain `GET` requests only (no `Range` header), which avoids CORS preflight for cross-origin deployments.

See [`../specs/chunked-disk-image-format.md`](../specs/chunked-disk-image-format.md) and the publisher CLI at
[`tools/image-chunker/`](../decisions/README.md).

Endpoints:

- `GET/HEAD /v1/images/:imageId/chunked/manifest`
- `GET/HEAD /v1/images/:imageId/chunked/chunks/:chunkIndex` (`:chunkIndex` can be `42` or a `.bin` filename like `00000042.bin` / `00.bin`)

If CloudFront is configured, these endpoints redirect to CloudFront (stable URLs for cookie mode; signed URLs for url mode).

`GET /v1/images/:imageId/stream-url` may include a `chunked` section:

```json
{
  "chunked": {
    "delivery": "chunked",
    "manifestUrl": "..."
  }
}
```

Notes:

- In `CLOUDFRONT_AUTH_MODE=cookie`, `manifestUrl` points directly at the CloudFront URL for `manifest.json`.
- In `CLOUDFRONT_AUTH_MODE=url`, `manifestUrl` points at the gateway endpoint (`/v1/images/:id/chunked/manifest`) so that
  chunk URLs resolved relative to the manifest (e.g. `new URL("chunks/00000000.bin", manifestUrl)`) also hit the gateway,
  which then redirects each request to a per-object signed CloudFront URL. This avoids needing query-string auth to propagate
  through relative URL resolution.
- disable compression / transformations (no `Content-Encoding` other than `identity`)
- include the streaming-safe headers described in `../areas/storage.md`:
  - `Cache-Control: no-transform`
  - `Content-Type: application/octet-stream`
  - `X-Content-Type-Options: nosniff`
  - `Cross-Origin-Resource-Policy: same-site` (or `cross-origin` depending on deployment)
  - CORS headers (if the app is cross-origin to the disk URL)

See `../areas/storage.md` for a concrete CloudFront response headers policy.

---

### Purged tree document: poc/README.md

## Proofs of concept (`poc/`)

This directory contains **small, self-contained proofs-of-concept** used to validate browser/platform constraints.

These are **not production entrypoints**. The production browser host app lives in `web/`.

### Contents

- `browser-memory/` – SharedArrayBuffer + `WebAssembly.Memory` memory model PoC.
  - Run: `node poc/browser-memory/server.mjs` then open the printed URL.

---

### Purged tree document: prototype/README.md

## Prototypes (`prototype/`)

This directory contains **larger prototypes** and RFC companion code that is useful for experimentation and design iteration.

These are **not production components**. The production browser host app lives in `web/`.

### Contents

- `nt-arch-rfc/` – Networking Architecture RFC prototype (see `../areas/networking.md`).

---

### Purged tree document: prototype/legacy-win7-aerogpu-1ae0/README.md

## Legacy AeroGPU 1AE0 prototype (archived)

This directory contains a **deprecated** AeroGPU prototype stack that used PCI vendor ID **1AE0**.
It predates (and does **not** match) the supported AeroGPU device models/protocols in this
repository.

On Windows 7 x64, the archived 1AE0 Windows driver package is also **not WOW64-complete**
(it does not ship/install an x86 UMD), so 32-bit D3D9 apps will fail.

The checked-in driver projects in this archive are also **x64-only** (no x86 builds).

Supported AeroGPU ABIs in this repo:

- **Legacy bring-up ABI (1AED; deprecated)**: `drivers/aerogpu/protocol/legacy/aerogpu_protocol_legacy.h` and the emulator
  device `crates/emulator/src/devices/pci/aerogpu_legacy.rs` (feature `emulator/aerogpu-legacy`).
- **Current versioned ABI (A3A0)**: `drivers/aerogpu/protocol/aerogpu_{pci,ring,cmd}.h` and the
  emulator device `crates/emulator/src/devices/pci/aerogpu.rs`.

Contents:

- `prototype/legacy-win7-aerogpu-1ae0/guest/windows/`: archived Windows 7 WDDM 1.1 + D3D9 driver stack targeting the 1AE0 prototype.
- The matching host-side 1AE0 device model is **not** part of the current emulator codebase.
  (Only the 1AED legacy and A3A0 versioned AeroGPU devices are supported.) If you need the 1AE0
  host-side prototype for archaeology, retrieve it from git history.

Do not use this prototype for new development.

For the supported Win7 driver package + install workflow, start at:
`drivers/aerogpu/packaging/win7/README.md` (and `drivers/aerogpu/build/stage_packaging_win7.cmd`).

---

### Purged tree document: guest/windows/README.md

## Legacy AeroGPU Windows 7 driver stack (archived)

This directory intentionally contains only small **pointer/stub** files (this README and a
redirecting `docs/driver_install.md` and stub `inf/aerogpu.inf`), not a real driver source tree.

Historically, the repo had a `guest/windows/` Win7 AeroGPU prototype driver tree. It has since
been **archived** to avoid accidental installs of an incomplete/obsolete package.

### Supported Win7 AeroGPU drivers (recommended)

Use the maintained Win7 driver package under:

- [`drivers/aerogpu/packaging/win7/README.md`](../decisions/README.md)

### Archived prototype (historical reference only)

The archived prototype sources and its old install guide live under:

- ``prototype/legacy-win7-aerogpu-1ae0/guest/windows/``

Important limitations of the archived prototype:

- Targets the deprecated AeroGPU prototype / bring-up PCI identities (not the supported A3A0 device contract).
- On Windows 7 x64 it is **not WOW64-complete** (no x86 UMD), so **32-bit D3D9 apps will fail**.

---

### Purged tree document: tools/aero-gateway-rs/README.md

## aero-gateway-rs (legacy / diagnostic)

This directory contains a legacy Rust/Axum gateway prototype that only implements the historical
`/tcp?target=<host>:<port>` WebSocket tunnel (plus a small `/admin` surface and optional on-disk
capture).

It is **not** the production Aero Gateway contract.

The maintained, CI-tested gateway implementation lives in `services/gateway/` (Node/TypeScript),
and the public contract is documented in:

- `../specs/gateway-api.md`
- `services/gateway/openapi.yaml`

### Security warning

This prototype is **not production-hardened** (no session cookie/auth contract, no origin/CORS
enforcement, no destination allow/deny policy, etc.). Treat it as an **unsafe open proxy** and do
not expose it to untrusted networks.

### Run

```bash
cd tools/aero-gateway-rs
cargo run --locked
```

This tool is intentionally **not** a Rust workspace member (see the repo root `Cargo.toml`) so it
does not increase default `cargo build/test` surface area. Build/run it explicitly from this
directory.

Environment variables:

- `AERO_GATEWAY_BIND_ADDR` (default: `127.0.0.1:8080`)
- `ADMIN_API_KEY` (enables `/admin/*`)
- `CAPTURE_DIR`, `CAPTURE_MAX_BYTES`, `CAPTURE_MAX_FILES` (enables capture)

---

### Purged tree document: deploy/README.md

## Aero deployment (TLS + COOP/COEP at the edge)

This directory contains **production** and **local-dev** deployment artifacts that:

1) Terminate TLS (HTTPS/WSS) at the edge
2) Enforce **cross-origin isolation** headers (COOP/COEP/CORP) required for:
   - `SharedArrayBuffer` + WASM threads
   - some high-performance browser execution patterns
3) Set additional hardening headers (CSP, Referrer-Policy, Permissions-Policy, etc.)
4) Reverse-proxy backend HTTP APIs and WebSocket upgrades (e.g. `/tcp`, `/tcp-mux`, `/l2` (legacy alias: `/eth`)) to backend services
5) Reverse-proxy WebRTC signaling + ICE discovery endpoints (e.g. `/webrtc/ice`) to the UDP relay

The recommended topology is **single-origin**:

```
Browser  ──HTTPS/WSS──▶  Caddy (edge)  ──HTTP/WS──▶  aero-gateway
                     │                ──HTTP/WS──▶  aero-l2-proxy
                     │                ──HTTP/WS──▶  aero-webrtc-udp-relay
                     │
                     └──UDP (ICE + relay data)────▶  aero-webrtc-udp-relay (published UDP range)

Same-origin for UI + APIs (no CORS needed).
```

### Optional: UDP relay service (WebRTC + WebSocket fallback)

The gateway (`services/gateway`) covers **TCP** (WebSocket) and **DNS-over-HTTPS**. Guest **UDP** requires a separate relay service:

- [`proxy/webrtc-udp-relay`](../../proxy/webrtc-udp-relay/) — WebRTC DataChannel (`label="udp"`) with a `GET /udp` WebSocket fallback, using the versioned v1/v2 datagram framing in [`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md).
  - Security note: `GET /webrtc/ice` responses may include sensitive TURN credentials (especially TURN REST ephemeral creds) and are explicitly **non-cacheable** (`Cache-Control: no-store`, `Pragma: no-cache`, `Expires: 0`). Reverse proxies should preserve these headers.

To integrate the relay with the gateway (recommended for production):

1. Deploy the relay somewhere reachable by the browser.
2. Configure the gateway with `UDP_RELAY_BASE_URL` (accepts `http(s)://` or `ws(s)://`) and a matching relay auth mode (`none`, `api_key`, or `jwt`).
3. The gateway’s `POST /session` response will include an `udpRelay` field (base URL + endpoints + short‑lived token), and clients can optionally refresh the token via `POST /udp-relay/token`.

### Files

- `deploy/docker-compose.yml` – runs:
  - `aero-proxy` (Caddy) on `:80/:443`
  - `aero-gateway` (`services/gateway`) on the internal docker network
  - `aero-l2-proxy` (`crates/aero-l2-proxy`) on the internal docker network
  - `aero-webrtc-udp-relay` (`proxy/webrtc-udp-relay`) for WebRTC UDP relay (HTTP behind Caddy, UDP published)
  - (optional) `coturn` TURN server via compose profile
- `deploy/caddy/Caddyfile` – TLS termination, COOP/COEP headers, reverse proxy rules
- `deploy/scripts/smoke.sh` – builds + boots the compose stack and asserts key headers
- `deploy/static/index.html` – a small **smoke test page** to validate `window.crossOriginIsolated` and basic networking wiring (`/session`, `/tcp`, `/l2`)
- `deploy/k8s/` – Kubernetes/Helm deployment for `aero-gateway` with Ingress TLS + COOP/COEP/CSP headers

For CSP details and tradeoffs (including why Aero needs `'wasm-unsafe-eval'` for WASM-based JIT),
see: `../areas/security.md`.

### Production-ready vs examples

This directory intentionally includes both **copy/paste-ready** configs and **reference-only**
templates.

Production-ready building blocks:

- `deploy/docker-compose.yml` + `deploy/caddy/Caddyfile` – single-host deployments (VM/bare metal)
- `deploy/k8s/chart/aero-gateway/` – Kubernetes Helm chart for the gateway + Ingress headers

Examples / reference-only:

- `deploy/static/` – smoke-test frontend (not the real UI)
- `deploy/nginx/` – nginx examples (useful if you don't want Caddy)
- `deploy/k8s/aero-storage-server/` – optional disk/image service templates (not required for the gateway)
- Static-host templates (browser host app):
  - `_headers` (Cloudflare Pages / Netlify-style):
    - `apps/web/public/_headers` (copied to `dist/_headers` on build)
    - `deploy/cloudflare-pages/_headers` (copy/paste template variant)
  - Netlify (`netlify.toml`):
    - `netlify.toml` (repo root; build config + header rules)
    - `deploy/netlify.toml` (headers-only template)
  - Vercel (`vercel.json`):
    - `vercel.json` (repo root; build config + header rules)
    - `deploy/vercel.json` (headers-only template)

The header values are centralized in `scripts/headers.json` (exported via `scripts/security_headers.mjs`).
CI enforces consistency via `scripts/ci/check-security-headers.mjs`.

### CI validation (Terraform + Helm)

CI validates the deployment artifacts under:

- `infra/` (Terraform formatting + validation)
- `deploy/k8s/` (Helm lint + template rendering + Kubernetes schema validation)
- Compose deployment manifests (labels + `docker compose config` validation; see `scripts/ci/check-deploy-manifests.mjs`)

Reproduce locally:

```bash
# Full reproduction (requires: node + docker compose + terraform + tflint + helm + kubeconform)
bash ./scripts/ci/check-iac.sh
# Or, if you have `just` installed:
#   just check-iac

# Deploy manifest labelling/hygiene (requires docker compose; fails on compose warnings)
node scripts/ci/check-deploy-manifests.mjs

# Terraform (requires `terraform`; CI also runs `tflint`)
cd infra/aws-s3-cloudfront-range
terraform fmt -check -recursive
terraform init -backend=false -input=false -lockfile=readonly
terraform validate

# Optional: extra linting (requires `tflint`)
tflint --init
tflint

# Helm/Kubernetes (requires `helm` + `kubeconform`)
CHART=deploy/k8s/chart/aero-gateway
for values in \
  values-dev.yaml \
  values-prod.yaml \
  values-traefik.yaml \
  values-prod-certmanager.yaml \
  values-prod-certmanager-issuer.yaml \
  values-prod-appheaders.yaml; do
  helm lint "$CHART" --strict --kube-version 1.28.0 -f "$CHART/$values"
done

for values in \
  values-dev.yaml \
  values-prod.yaml \
  values-traefik.yaml \
  values-prod-certmanager.yaml \
  values-prod-certmanager-issuer.yaml \
  values-prod-appheaders.yaml; do
  out="/tmp/aero-${values%.yaml}.yaml"
  helm template aero-gateway "$CHART" -n aero --kube-version 1.28.0 -f "$CHART/$values" > "$out"
  kubeconform -strict \
    -schema-location default \
    -schema-location "https://raw.githubusercontent.com/datreeio/CRDs-catalog/main/{{.Group}}/{{.ResourceKind}}_{{.ResourceAPIVersion}}.json" \
    -kubernetes-version 1.28.0 -summary "$out"
done
```

### Production DNS requirements

To use public, browser-trusted certificates (Let’s Encrypt via Caddy), you need:

- An **A** record pointing your domain at your server IPv4
- (Optional) An **AAAA** record for IPv6
- Ports **80/tcp** and **443/tcp** reachable from the public internet

Example:

| Type | Name | Value |
|------|------|-------|
| A | `aero.example.com` | `203.0.113.10` |
| AAAA | `aero.example.com` | `2001:db8::10` |

### Environment variables

Set these in your shell or a `.env` file next to `deploy/docker-compose.yml`.

Quick start:

```bash
cp deploy/.env.example deploy/.env
# For production, set a strong SESSION_SECRET explicitly (recommended).
# If unset, `deploy/docker-compose.yml` generates and persists a random secret in a Docker volume
# (sessions survive restarts until `docker compose down -v`).
# Example: SESSION_SECRET=$(openssl rand -hex 32)
```

- `AERO_DOMAIN` (default: `localhost`)
  - `localhost` for local dev
  - `aero.example.com` (or similar) for production
- `AERO_GATEWAY_IMAGE` (default: `aero-gateway:dev`)
  - By default, `deploy/docker-compose.yml` builds `services/gateway` from source.
  - For production, prefer a published image and remove the compose `build:` stanza (or override it).
- `AERO_GATEWAY_GIT_SHA` (default: `dev`)
  - Optional build arg used to populate `GET /version` in `services/gateway`.
- `AERO_L2_PROXY_IMAGE` (default: `aero-l2-proxy:dev`)
  - By default, `deploy/docker-compose.yml` builds `crates/aero-l2-proxy` from source.
  - For production, prefer a published image and remove/override the compose `build:` stanza.
- `AERO_GATEWAY_UPSTREAM` (default: `aero-gateway:8080`)
  - Only change if your gateway listens on a different port inside docker.
- `AERO_L2_PROXY_UPSTREAM` (default: `aero-l2-proxy:8090`)
  - Only change if your L2 proxy listens on a different port inside docker.
- `AERO_L2_ALLOWED_ORIGINS_EXTRA` (default: empty)
  - Optional comma-prefixed origins appended to the L2 proxy Origin allowlist.
  - Example: `,https://localhost:5173`
- `AERO_L2_AUTH_MODE` (default: `none`)
  - Authentication mode for `/l2` (handled by `crates/aero-l2-proxy`).
  - Supported values: `none`, `session`, `token`, `session_or_token`, `session_and_token`, `jwt`, `cookie_or_jwt`.
    - Legacy aliases: `cookie`, `api_key`, `cookie_or_api_key`, `cookie_and_api_key`.
- `AERO_L2_SESSION_SECRET` (optional override)
  - Secret for validating the `aero_session` cookie when `AERO_L2_AUTH_MODE=session|cookie_or_jwt|session_or_token|session_and_token`.
  - `crates/aero-l2-proxy` reads this from `AERO_GATEWAY_SESSION_SECRET` (preferred) and falls back to
    `SESSION_SECRET` / `AERO_L2_SESSION_SECRET` (legacy), so the deploy stack can share one secret
    across both services.
- `AERO_L2_API_KEY` / `AERO_L2_JWT_SECRET` (optional)
  - Credentials for `AERO_L2_AUTH_MODE=token|jwt|cookie_or_jwt|session_or_token|session_and_token`.
  - Credentials can be delivered via query params:
    - Token auth: `?token=...` (preferred) (or `?apiKey=...` for compatibility)
    - JWT auth: `?token=...`
    or an additional `Sec-WebSocket-Protocol` entry `aero-l2-token.<value>` (offered alongside
    `aero-l2-tunnel-v1`; prefer this form when possible to avoid putting secrets in URLs/logs).
- `AERO_L2_TOKEN` (optional, legacy)
  - Legacy alias for token auth.
  - `deploy/docker-compose.yml` defaults `AERO_L2_AUTH_MODE=none`, so `AERO_L2_TOKEN` has no effect unless you
    explicitly enable token auth (e.g. `AERO_L2_AUTH_MODE=token|session_or_token|session_and_token`).
  - Accepted as a fallback value for `AERO_L2_API_KEY` when `AERO_L2_AUTH_MODE=token|session_or_token|session_and_token`.
  - Ignored when `AERO_L2_AUTH_MODE` is set to `session` (legacy alias: `cookie`), `jwt`, `cookie_or_jwt`, or `none`.
- `AERO_L2_CAPTURE_DIR` (default: empty / unset)
  - When set, `aero-l2-proxy` writes per-tunnel PCAPNG capture files into this directory (debugging only).
  - The directory must be writable by the container user (the `aero-l2-proxy` image runs as non-root). `/tmp/...` is
    a safe default in most environments.
  - **Privacy warning:** captures contain raw network traffic and may include sensitive user data (DNS queries,
    plaintext protocols, credentials, etc.). Treat capture files as secrets and avoid enabling capture on
    internet-exposed production deployments unless you have an explicit retention/privacy plan.
  - Note: capture files are written inside the container filesystem. To persist captures across container restarts,
    bind-mount a host directory (or use a named volume) and set `AERO_L2_CAPTURE_DIR` to the mounted path.
- `AERO_L2_CAPTURE_MAX_BYTES` (default in compose: `67108864` / 64 MiB)
  - Max capture file size per tunnel session (includes PCAPNG overhead; `0` disables the cap).
- `AERO_L2_CAPTURE_FLUSH_INTERVAL_MS` (default in compose: `1000`)
  - Flush interval for capture writers (`0` disables periodic flushing; capture is flushed on close). Larger values
    reduce disk I/O but may lose the last buffered packets if the process crashes.
- `AERO_WEBRTC_UDP_RELAY_IMAGE` (default: `aero-webrtc-udp-relay:dev`)
  - When unset, docker compose builds the UDP relay from `proxy/webrtc-udp-relay/`.
- `AERO_WEBRTC_UDP_RELAY_UPSTREAM` (default: `aero-webrtc-udp-relay:8080`)
  - Only change if your relay listens on a different port inside docker.
- `AERO_WEBRTC_UDP_RELAY_ALLOWED_ORIGINS_EXTRA` (default: empty)
  - Optional comma-prefixed origins appended to the relay `ALLOWED_ORIGINS` allowlist.
  - Example: `,https://localhost:5173`
- `AERO_HSTS_MAX_AGE` (default: `0`)
  - `0` disables HSTS (good for local dev)
  - Recommended production value: `31536000` (1 year)
- `AERO_FRONTEND_ROOT` (default: `./static`)
  - Which directory Caddy serves as `/` (mounted at `/srv` in the container).
  - Recommended: `../dist` (after building the real frontend)
- `AERO_CSP_CONNECT_SRC_EXTRA` (default: empty)
  - Optional additional origins to allow in the Caddy Content Security Policy `connect-src`.
  - Use this if the frontend needs to connect to a separate origin for networking (e.g. a TCP proxy service).
  - Example: `AERO_CSP_CONNECT_SRC_EXTRA="https://proxy.example.com wss://proxy.example.com"`

Gateway environment variables (used by `services/gateway` and passed through in
`deploy/docker-compose.yml`):

- `PUBLIC_BASE_URL` (default in compose: `https://${AERO_DOMAIN}`)
  - Used to derive the default `ALLOWED_ORIGINS` allowlist.
- `SESSION_SECRET` (strongly recommended for production)
  - HMAC secret used to sign the `aero_session` cookie minted by `POST /session`.
  - Used to authenticate privileged endpoints like `/tcp`, and `/l2` when the L2 proxy is configured for
    session-cookie auth (`AERO_L2_AUTH_MODE=session|cookie_or_jwt|session_or_token|session_and_token`).
  - If unset, the deploy stack generates and persists a random secret in a Docker volume (sessions survive
    restarts until `docker compose down -v`).
  - When using session-cookie auth for the L2 tunnel (`AERO_L2_AUTH_MODE=session` / `cookie_or_jwt` / `session_or_token` / `session_and_token`), `crates/aero-l2-proxy`
    must share the same signing secret so it can validate the `aero_session` cookie minted by the gateway.
- `ALLOWED_ORIGINS` (optional, comma-separated)
  - Set explicitly if you need to allow additional origins (e.g. a dev server).
- `TRUST_PROXY` (default in compose: `1`)
  - Set to `1` only when the gateway is reachable **only** via the reverse proxy.
  - Required if you want `request.ip` / rate limiting to use `X-Forwarded-For`.
- `CROSS_ORIGIN_ISOLATION` (default in compose: `0`)
  - Set to `1` only if you are not injecting COOP/COEP headers at the edge proxy.

#### WebRTC UDP relay configuration

The UDP relay (`proxy/webrtc-udp-relay`) has two networking surfaces:

- **HTTP (same-origin)**: proxied behind Caddy for:
  - `GET /webrtc/ice`
  - `GET /webrtc/signal` (WebSocket)
  - `POST /webrtc/offer` and `POST /offer`
  - `GET /udp` (WebSocket UDP fallback; same datagram framing as the WebRTC DataChannel)
- **UDP (not proxyable)**: ICE + data plane UDP ports must be reachable by browser clients.

Defaults in `deploy/docker-compose.yml`:

- `WEBRTC_UDP_PORT_MIN=50000`
- `WEBRTC_UDP_PORT_MAX=50100`
- Host publishing: `50000-50100/udp`

If `docker compose up` fails with a message like `port is already allocated`, pick a different range by
overriding `WEBRTC_UDP_PORT_MIN/MAX` in `deploy/.env` (the compose file uses these vars for both the
container env and the published UDP port range).

If you change the ICE port range, you must update:

1) The env vars (`WEBRTC_UDP_PORT_MIN/MAX`), and
2) The published UDP port range (firewall + docker `ports:`).

The relay supports authentication via `AUTH_MODE` (and `API_KEY`/`JWT_SECRET`). These values
are also used by `services/gateway` to mint `udpRelay.token` in `POST /session` so the
frontend can discover how to authenticate to the relay.

##### Optional: WebRTC DataChannel / SCTP hardening (oversized message DoS)

The relay enforces `MAX_DATAGRAM_PAYLOAD_BYTES` / `L2_MAX_MESSAGE_BYTES` at the application layer, but
malicious peers can attempt to send extremely large WebRTC DataChannel messages that may be fully buffered by pion
before `DataChannel.OnMessage` is invoked.

To mitigate this, the relay supports these env vars (passed through by `deploy/docker-compose.yml` and configurable via `deploy/.env`):

- `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` (0 = auto): advertised max message size via SDP `a=max-message-size` (best-effort for compliant peers).
- `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES` (0 = auto): hard receive-side buffering cap in pion/SCTP (must be ≥ `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` and ≥ `1500`).

See `proxy/webrtc-udp-relay/README.md` for details (including how the auto-derived defaults are computed).

##### Optional: WebRTC session connect timeout (half-open session leak mitigation)

To avoid leaking server-side PeerConnections due to half-open WebRTC sessions (misbehaving clients / DoS), the relay enforces a connect timeout and closes PeerConnections that never reach a connected state:

- `WEBRTC_SESSION_CONNECT_TIMEOUT` (default `30s`)

This setting is also passed through by `deploy/docker-compose.yml` and configurable via `deploy/.env`. Setting it too low can break slow networks / delayed ICE or TURN negotiation.

The relay also enforces a **UDP destination policy** (to prevent accidental open-proxy deployments).
By default, `proxy/webrtc-udp-relay` uses `DESTINATION_POLICY_PRESET=production` (**deny by default**),
so you must configure an allowlist (CIDRs/ports) before UDP relay traffic will flow.

For local development/testing, you can set `DESTINATION_POLICY_PRESET=dev` to allow by default.

##### Optional: inbound UDP filtering (NAT behavior)

By default, the relay applies **inbound filtering** so it only forwards inbound UDP from remote
address+port tuples that the guest previously sent to (`UDP_INBOUND_FILTER_MODE=address_and_port`).

You can switch to full-cone behavior with:

- `UDP_INBOUND_FILTER_MODE=any` (**less safe**; accepts inbound UDP from any remote)

Additional knobs:

- `UDP_REMOTE_ALLOWLIST_IDLE_TIMEOUT` (default: `UDP_BINDING_IDLE_TIMEOUT`) — expire inactive allowlist entries
- `MAX_ALLOWED_REMOTES_PER_BINDING` — cap the number of tracked remotes per UDP binding (DoS hardening)

See:

- `proxy/webrtc-udp-relay/README.md` (authoritative)

#### Optional: L2 tunnel over WebRTC (relay bridging)

`proxy/webrtc-udp-relay` can also bridge a **fully reliable and ordered** WebRTC DataChannel
labeled `l2` to an
L2 tunnel backend WebSocket (typically `aero-l2-proxy`):

```
browser DataChannel "l2"  <->  aero-webrtc-udp-relay  <->  aero-l2-proxy /l2
```

The `l2` DataChannel must be configured as:

- `ordered = true`
- do **not** set `maxRetransmits` / `maxPacketLifeTime` (partial reliability)

This is useful when you want to carry the L2 tunnel over a UDP-based transport (WebRTC) for
experimentation under loss/NAT traversal.

To enable it in the compose stack, set in `deploy/.env`:

```bash
L2_BACKEND_WS_URL=ws://aero-l2-proxy:8090/l2
```

The relay can forward the client’s **Origin** and **AUTH_MODE credential** (JWT/token) to the
backend dial. Relevant env vars are documented in `proxy/webrtc-udp-relay/README.md`, including:

- `L2_BACKEND_AUTH_FORWARD_MODE=none|query|subprotocol`
- `L2_BACKEND_FORWARD_ORIGIN=1`
- `L2_BACKEND_FORWARD_AERO_SESSION=1` (recommended for `AERO_L2_AUTH_MODE=session`; forwards `Cookie: aero_session=...` captured from signaling)
- `L2_BACKEND_ORIGIN_OVERRIDE=https://example.com` (optional)

Note: the backend `/l2` endpoint must be configured to accept whatever auth material the relay forwards. For
session-cookie `/l2` (`AERO_L2_AUTH_MODE=session`; legacy alias: `cookie`), enable
`L2_BACKEND_FORWARD_AERO_SESSION=1` so WebRTC L2 bridging continues to work while preserving session-cookie auth.

### Optional TURN server (coturn profile)

Some client networks require TURN for reliable UDP connectivity. This repo includes an opt-in `coturn` service:

```bash
docker compose -f deploy/docker-compose.yml --profile turn up --build
```

Ports published by the TURN profile:

- `3478/udp` (TURN listening port)
- `49152-49200/udp` (TURN relay range; must match `proxy/webrtc-udp-relay/turn/turnserver.conf`)

To have browsers actually use TURN, configure the relay’s ICE server list to include a TURN URL
pointing at your host (often the same as `AERO_DOMAIN`):

- `AERO_ICE_SERVERS_JSON`, or
- `AERO_TURN_URLS` + `AERO_TURN_USERNAME` + `AERO_TURN_CREDENTIAL`

> Note: `turnserver.conf` defaults to `user=aero:aero` for local dev. Change credentials and
> `external-ip=...` before exposing TURN to the public internet.

### Local dev (self-signed TLS)

Run:

```bash
docker compose -f deploy/docker-compose.yml up --build
```

Then open:

- `https://localhost/`

You should see the smoke test page.

> Note: Caddy serves HTTPS with HTTP/2 enabled automatically when using TLS.

#### Trusting the certificate (recommended)

For `localhost`, Caddy uses an internal CA. Browsers may require you to trust
that CA for the origin to be treated as fully secure.

To export the Caddy local root CA:

```bash
docker compose -f deploy/docker-compose.yml cp aero-proxy:/data/caddy/pki/authorities/local/root.crt ./caddy-local-root.crt
```

Then import `./caddy-local-root.crt` into your OS/browser trust store.

### Verifying cross-origin isolation

#### SharedArrayBuffer enablement checklist

To reliably get `SharedArrayBuffer` + WASM threads working in production:

- [ ] The page is served from a **secure context** (`https://` in production)
- [ ] The main document response includes **COOP + COEP**:
  - [ ] `Cross-Origin-Opener-Policy: same-origin`
  - [ ] `Cross-Origin-Embedder-Policy: require-corp`
- [ ] Recommended additional hardening headers are present:
  - [ ] `Cross-Origin-Resource-Policy: same-origin`
  - [ ] `Origin-Agent-Cluster: ?1`
- [ ] All subresources (scripts/wasm/workers) are **same-origin**, or explicitly
      CORS/CORP-enabled
- [ ] No mixed content (no `http://` subresources on an `https://` page)

#### Check the headers

```bash
# If you haven't trusted the local Caddy CA yet, add `-k` (insecure) or trust the
# CA as described below.
curl -kI https://localhost/
```

Expect:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp`
- `Cross-Origin-Resource-Policy: same-origin`
- `Origin-Agent-Cluster: ?1`

#### Check in the browser

Open DevTools Console:

```js
window.crossOriginIsolated === true
```

Also check:

```js
typeof SharedArrayBuffer !== "undefined"
```

If `crossOriginIsolated` is `false`, the most common causes are:

- Missing COOP/COEP headers on the **HTML document** response
- One or more subresources (scripts/wasm/workers) being loaded cross-origin
  without proper `CORP` or CORS headers
- TLS is not considered secure (certificate not trusted, mixed content, etc.)

### WebSocket proxy validation (WSS)

The edge proxy is configured to forward WebSocket upgrades for `/tcp`.

You can validate that the TLS + upgrade path works with a CLI client like
[`wscat`](https://github.com/websockets/wscat) or [`websocat`](https://github.com/vi/websocat):

```bash
# If you haven't trusted the local Caddy CA yet, you may need:
#   NODE_TLS_REJECT_UNAUTHORIZED=0
#
# Step 1: create a cookie-backed session (copy the `aero_session=...` value from Set-Cookie).
curl -k -i -X POST https://localhost/session -H 'content-type: application/json' -d '{}'
#
# Canonical (v1):
#   /tcp?v=1&host=<hostname-or-ip>&port=<port>
NODE_TLS_REJECT_UNAUTHORIZED=0 npx wscat \
  -c "wss://localhost/tcp?v=1&host=example.com&port=80" \
  -H "Cookie: aero_session=<paste-from-Set-Cookie>" \
  -o https://localhost

# Compatibility form (legacy; also supported by the gateway):
# NODE_TLS_REJECT_UNAUTHORIZED=0 npx wscat \
#   -c "wss://localhost/tcp?v=1&target=example.com:80" \
#   -H "Cookie: aero_session=<paste-from-Set-Cookie>" \
#   -o https://localhost
```

If you see a successful handshake but the connection immediately closes, the
gateway may be rejecting the query parameters or target.

> Note: `/tcp` is a privileged endpoint; the gateway rejects upgrades without an `aero_session` cookie.

### L2 tunnel proxy (/l2; legacy alias /eth)

The **L2 tunnel proxy** (`aero-l2-proxy`) provides an Ethernet (L2) tunnel over
WebSocket:

- The browser connects to `wss://<AERO_DOMAIN>/l2`
- Legacy alias: `wss://<AERO_DOMAIN>/eth` (prefer `/l2` for new clients)
- Caddy proxies the WebSocket upgrade to the `aero-l2-proxy` container
- The connection uses subprotocol: `aero-l2-tunnel-v1`

#### L2 tunnel auth (session cookie)

`/l2` enforces an Origin allowlist by default. For production deployments you should also enable
authentication. The recommended mode for same-origin browser clients is session-cookie auth
(`AERO_L2_AUTH_MODE=session`; legacy alias: `cookie`):

- `aero-l2-proxy` validates the `aero_session` cookie minted by the gateway (`POST /session`).
  - Browser WebSocket handshakes include cookies automatically for same-origin connections.
  - CLI clients must provide `Cookie: aero_session=...` on the WebSocket upgrade.

For non-browser clients / internal bridges, you can switch to token-based auth
(`AERO_L2_AUTH_MODE=token|jwt`), accept either a session cookie or a token/JWT
(`AERO_L2_AUTH_MODE=session_or_token|cookie_or_jwt`), or require both
(`AERO_L2_AUTH_MODE=session_and_token`; legacy alias: `cookie_and_api_key`).
Credentials can be provided via:

- Query params: `?token=<value>` (preferred; `?apiKey=<value>` for compatibility)
- Preferred (avoids secrets in URLs/logs): offer an additional `Sec-WebSocket-Protocol` entry
  `aero-l2-token.<value>` (offered alongside `aero-l2-tunnel-v1`)

To disable auth (local dev only; NOT recommended for internet-exposed deployments), set
`AERO_L2_AUTH_MODE=none` in `deploy/.env`.

See `deploy/.env.example` for copy/paste configuration examples.

Endpoint discovery note: browser clients should treat the gateway as the canonical bootstrap API and
avoid hardcoding `/l2`. The `POST /session` response includes `endpoints.l2` (a same-origin path)
and `limits.l2` (payload size caps) so the frontend can connect and tune buffering without baking in
paths or protocol constants.

This endpoint is intended for the “Option C” architecture (tunneling Ethernet frames to a server-side
network stack / NAT / policy layer).

#### Run locally

```bash
docker compose -f deploy/docker-compose.yml up --build
```

#### Validate the upgrade (WSS)

> Note: `aero-l2-proxy` enforces an Origin allowlist by default. CLI clients must
> send an `Origin` header that matches the allowlist (in this deploy stack:
> `https://localhost` unless you changed `AERO_DOMAIN`).

Using `wscat`:

```bash
# Step 1: create a cookie-backed session (copy the `aero_session=...` value from Set-Cookie):
curl -k -i -X POST https://localhost/session -H 'content-type: application/json' -d '{}'
#
# Step 2: connect to /l2 with the Cookie header:
NODE_TLS_REJECT_UNAUTHORIZED=0 npx wscat \
  -c "wss://localhost/l2" \
  -s aero-l2-tunnel-v1 \
  -H "Cookie: aero_session=<paste-from-Set-Cookie>" \
  -o https://localhost

# Token auth (optional): provide a credential via query param:
# NODE_TLS_REJECT_UNAUTHORIZED=0 npx wscat \
#   -c "wss://localhost/l2?token=<credential>" \
#   -s aero-l2-tunnel-v1 \
#   -o https://localhost
```

Using `websocat`:

```bash
# Some `websocat` versions do not send an Origin header by default. If you get a 403,
# add it explicitly (see `websocat --help` for the exact flag in your version).
websocat --insecure --protocol aero-l2-tunnel-v1 \
  -H "Origin: https://localhost" \
  -H "Cookie: aero_session=<paste-from-Set-Cookie>" \
  wss://localhost/l2
#
# Token/JWT auth:
# websocat --insecure --protocol aero-l2-tunnel-v1 \
#   -H "Origin: https://localhost" \
#   wss://localhost/l2?token=<credential>
```

#### Production note: egress policy

`aero-l2-proxy` is a **network egress surface**. For production deployments you should configure a
deny-by-default policy (ports/domains) to avoid exposing an open proxy.

Supported env vars include:

- `AERO_L2_ALLOWED_TCP_PORTS` (comma-separated)
- `AERO_L2_ALLOWED_UDP_PORTS` (comma-separated)
- `AERO_L2_ALLOWED_DOMAINS` / `AERO_L2_BLOCKED_DOMAINS` (comma-separated suffixes)
- `AERO_L2_ALLOW_PRIVATE_IPS=1` (dev-only; disables private/reserved IP blocking)
- `AERO_L2_MAX_FRAME_PAYLOAD` (default: `2048`; legacy alias: `AERO_L2_MAX_FRAME_SIZE`)
- `AERO_L2_MAX_CONTROL_PAYLOAD` (default: `256`)
  - Values must be positive integers; `0`/blank are treated as unset (defaults apply).
- `AERO_L2_MAX_UDP_FLOWS_PER_TUNNEL` (default: `256`; `0` disables)
- `AERO_L2_UDP_FLOW_IDLE_TIMEOUT_MS` (default: `60000`; `0` disables)
- `AERO_L2_STACK_MAX_TCP_CONNECTIONS` (default: `1024`)
- `AERO_L2_STACK_MAX_PENDING_DNS` (default: `1024`)
- `AERO_L2_STACK_MAX_DNS_CACHE_ENTRIES` (default: `10000`)
- `AERO_L2_STACK_MAX_BUFFERED_TCP_BYTES_PER_CONN` (default: `262144`)

See also: `../areas/networking.md` (production checklist).

### Networking smoke tests

```bash
# Gateway liveness (should return 200 + {"ok":true}):
curl -k https://localhost/healthz

# UDP relay ICE discovery:
# - Default (AUTH_MODE=none): should return 200 + {"iceServers":[...]} with no credentials.
# - If AUTH_MODE=api_key: requires an API key (X-API-Key) matching API_KEY.
curl -k https://localhost/webrtc/ice
# Or (api_key mode):
# curl -k -H "X-API-Key: <API_KEY>" https://localhost/webrtc/ice

# Session bootstrap (sets a Secure cookie when behind the TLS proxy and returns relay config):
curl -k -i -X POST https://localhost/session -H 'content-type: application/json' -d '{}'
```

#### `/session` and UDP relay integration notes

`services/gateway` implements `POST /session` as the session bootstrap endpoint.
It sets the `aero_session` cookie and returns a JSON payload that includes (when configured):

- `udpRelay.baseUrl` (expected to be the same origin in this deploy stack)
- `udpRelay.endpoints`:
  - `/webrtc/ice` (ICE server discovery)
  - `/webrtc/signal` (WebSocket signaling)
  - `/webrtc/offer` (HTTP offer/answer flow)
  - `/udp` (WebSocket UDP fallback)
- `udpRelay.token` / `udpRelay.expiresAt` when `AUTH_MODE` is enabled

The relay also exposes `POST /offer` as a legacy HTTP offer/answer endpoint; new clients should prefer `POST /webrtc/offer` (which is what the gateway advertises via `udpRelay.endpoints.webrtcOffer`).

In this deploy stack, the gateway is configured to advertise the relay at the same origin
(`https://$AERO_DOMAIN`) so the browser can stay single-origin and avoid CORS/cookie issues.

### CORS / origin strategy

#### Recommended (no CORS): same-origin UI + gateway

Serve the frontend and gateway through the same `https://AERO_DOMAIN` origin.
This is what the provided `Caddyfile` + compose setup enables.

Benefits:

- No CORS configuration required
- Simplest path to `crossOriginIsolated`
- WSS and WebRTC requirements are met by default (secure context)

#### Dev server (Vite) caveat

If you run a dev server like `http://localhost:5173`, you are **changing the
origin**, which introduces CORS requirements and can break cross-origin
isolation unless the dev server also sets COOP/COEP headers.

At minimum, your dev server must:

- Serve over a secure context (prefer `https://`)
- Send the same COOP/COEP/CORP headers on the HTML + JS/worker responses

For Vite, this is typically done by setting `server.headers` and enabling HTTPS.
This repo’s repo-root Vite app already includes these headers in `vite.harness.config.ts`
(and the legacy `web/` Vite app does as well via `web/vite.config.ts`).

If you need to call the gateway from a different origin (e.g. Vite dev server),
your gateway must also be configured with an explicit CORS allowlist (for
example, allowing `https://localhost:5173`). Prefer a strict allowlist over
`*`, especially once credentials or session tokens are involved.

#### Recommended dev workflow: keep a single origin

If you want hot-reload but still want **same-origin** + COOP/COEP enforcement,
run the Vite dev server separately and have Caddy proxy non-API routes to it.
That way the browser still sees a single `https://AERO_DOMAIN` origin.

### Serving your real frontend build

By default, `deploy/docker-compose.yml` mounts `deploy/static/` into the proxy
at `/srv` as a smoke test.

`deploy/caddy/Caddyfile` is tuned for Vite output:

- `/assets/*` gets long-lived caching (`Cache-Control: public, max-age=31536000, immutable`)
- everything else (including `index.html`) is served with `Cache-Control: no-cache`

To serve your real frontend:

1) Build it (example):

```bash
    npm ci
    npm run build:prod
```

2) Replace the volume mount in `deploy/docker-compose.yml`:

Set `AERO_FRONTEND_ROOT` (recommended; no compose edits required):

```bash
# in deploy/.env (or export it in your shell)
    AERO_FRONTEND_ROOT=../dist
```

3) Restart:

```bash
docker compose -f deploy/docker-compose.yml up --force-recreate
```

### Separate static hosting (frontend on a different origin)

The simplest/most robust setup is **single-origin** (serve static UI + gateway under the same host).
If you must host the frontend elsewhere (Netlify/Vercel/Cloudflare Pages), you must configure **both**:

1) **Frontend headers** (COOP/COEP + CSP)
2) **Gateway origin allowlist** (`PUBLIC_BASE_URL` / `ALLOWED_ORIGINS`)

Hosting templates in this repo:

- Netlify + Cloudflare Pages headers: `apps/web/public/_headers` (copied into `dist/_headers` on build)
- Netlify build config: `netlify.toml` (repo root)
- Vercel config: `vercel.json` (repo root)

When using a separate gateway origin, update the frontend CSP `connect-src` to include the gateway:

```
connect-src 'self' https://gateway.example.com wss://gateway.example.com
```

Then configure the gateway to allow the frontend origin:

- `PUBLIC_BASE_URL=https://gateway.example.com`
- `ALLOWED_ORIGINS=https://frontend.example.com` (comma-separated if multiple)

### Compose smoke check

To validate the compose stack end-to-end (build + security headers + `/healthz` + `/webrtc/ice` + `/dns-query` + `/tcp` + `/l2` WebSocket upgrades + `/udp` WebSocket datagram roundtrip + wasm MIME/caching), run:

```bash
bash deploy/scripts/smoke.sh
```

---

### Purged tree document: infra/aws-s3-cloudfront-range/README.md

## AWS S3 + CloudFront (HTTP Range) Terraform module

Reference infrastructure-as-code for hosting **large, immutable disk images** in a **private S3 bucket** and delivering them through a **CloudFront distribution** tuned for **HTTP Range requests** (partial downloads) and caching.

This is AWS-specific (uses CloudFront **Origin Access Control (OAC)**, not legacy OAI).

For a fully local Range + CORS validation setup (no AWS required), see
[`infra/local-object-store/README.md`](../decisions/README.md).

If you need a backend for **user uploads** (S3 multipart upload + CloudFront signed cookies/URLs),
see the reference implementation at ``services/image-gateway/``.

### Architecture

- **S3 bucket (private)**
  - Block all public access.
  - Default server-side encryption (SSE-S3 by default; optional SSE-KMS).
  - Optional versioning.
  - Lifecycle rules:
    - Abort incomplete multipart uploads (important for very large uploads).
    - Optional transition/expiration knobs.
  - Optional CORS configuration (secure default requires an explicit allowlist of origins; can be disabled if CORS is handled fully at CloudFront).
- **CloudFront distribution**
  - Origin is the S3 bucket, access controlled by **OAC**.
  - `http_version = http2and3` (HTTP/2 + HTTP/3).
  - Cache behavior for `/images/*`:
    - `GET`, `HEAD`, `OPTIONS`
    - Only `GET`/`HEAD` responses are cached by default (OPTIONS preflight is forwarded to origin; browsers cache via `Access-Control-Max-Age`).
    - Compression disabled (disk images are already compressed or not worth compressing).
    - Origin request policy forwards headers needed for **CORS preflight** (unless edge-handled preflight is enabled).
    - `Range`/`If-Range` are forwarded to S3 so byte-range reads work, but **are not included in the CloudFront cache key** (avoids cache fragmentation for random-access workloads).
    - Two cache policies: `immutable` (long TTL) vs `mutable` (short TTL), selectable via variable.
    - Optional CloudFront **response headers policy** for injecting CORS headers and optional security headers (e.g. `Cross-Origin-Resource-Policy`) at the edge.
      - Note: when configured via this module, these headers are **edge-enforced** (override origin values) to avoid per-object drift.
    - Optional CloudFront **Function** to answer CORS preflight (`OPTIONS`) at the edge for `/images/*`, avoiding OPTIONS to S3.

### CloudFront object size limits (important for very large disks)

CloudFront enforces a maximum object size per URL. If you plan to serve **very large** disk images, ensure your objects stay within CloudFront’s limits, or publish images as **chunk objects** instead of a single monolithic file.

For background and practical guidance (including a chunked-object strategy), see:

- [`../areas/storage.md`](../areas/storage.md)

### SSE-KMS note (if using `kms_key_arn`)

If you set `kms_key_arn` to enable SSE-KMS encryption for the bucket, ensure the referenced KMS key policy allows decrypt access for reads originating from this CloudFront distribution (otherwise CloudFront will receive `AccessDenied` when fetching objects from S3).

### Prerequisites

- Terraform **0.13+**
- Optional: [`tflint`](https://github.com/terraform-linters/tflint) (this repo pins config via `.tflint.hcl`)
- AWS credentials configured for Terraform (env vars, profile, or IAM role)
- AWS provider plugin will be downloaded during `terraform init`

### CI validation (Terraform + tflint)

CI runs:

- `terraform fmt -check -recursive`
- `terraform init -backend=false`
- `terraform validate`
- `tflint` (AWS ruleset)

To reproduce locally:

```bash
cd infra/aws-s3-cloudfront-range
terraform fmt -check -recursive
terraform init -backend=false -input=false -lockfile=readonly
terraform validate
tflint --init
tflint
```

Tip: from the repo root you can also run the full CI reproduction helper:

```bash
bash ./scripts/ci/check-iac.sh
# Or: just check-iac
```

### Quick start

```bash
cd infra/aws-s3-cloudfront-range
cp terraform.tfvars.example terraform.tfvars

# Edit terraform.tfvars, then:
terraform fmt
terraform init
terraform validate
terraform apply
```

After apply, Terraform prints:

- CloudFront distribution domain name
- S3 bucket name
- Recommended image base URL

### Example configuration

See `terraform.tfvars.example`.

### Uploading an image

Upload disk images under the `images/` prefix (so they are reachable at `/images/...`).

#### Immutable (recommended)

Use a **versioned path** (e.g. `windows7-v1.img`, `windows7/2026-01-10/disk.img`, etc) and set a long cache-control:

```bash
aws s3 cp ./windows7.img "s3://YOUR_BUCKET/images/windows7-v1.img" \
  --content-type "application/octet-stream" \
  --cache-control "public, max-age=31536000, immutable, no-transform"
```

#### Mutable

If you overwrite the same key (not recommended for very large artifacts), set a short cache-control and use the module’s `cache_policy_mode = "mutable"`:

```bash
aws s3 cp ./windows7.img "s3://YOUR_BUCKET/images/windows7-latest.img" \
  --content-type "application/octet-stream" \
  --cache-control "public, max-age=60, no-transform"
```

If clients might have cached old bytes, consider changing the object key instead of overwriting.

### Verifying Range delivery

Replace `BASE_URL` with the module output `image_base_url` (for example, `https://d123.cloudfront.net/images/`).

#### Verify object headers (HEAD)

```bash
curl -I "${BASE_URL}windows7-v1.img"
```

Look for headers similar to:

- `HTTP/2 200` (or `HTTP/3 200`)
- `Accept-Ranges: bytes`
- `Content-Length: ...`

#### Verify partial content (Range request)

```bash
curl -v -H "Range: bytes=0-1048575" "${BASE_URL}windows7-v1.img" -o /dev/null
```

Expected:

- `HTTP/2 206` (Partial Content)
- `Content-Range: bytes 0-1048575/…`

#### Verify CloudFront caching

Run the same request twice. On a cache hit you should see:

```bash
curl -I "${BASE_URL}windows7-v1.img" | grep -i '^x-cache:'
# X-Cache: Hit from cloudfront
```

The very first request is often `Miss from cloudfront`. Subsequent requests from the same edge location should become `Hit`.

#### Benchmark Range throughput + cache hit rate (recommended)

For deeper performance and cache analysis, use the repo’s Range harness:

```bash
# From the repo root:
node tools/range-harness/index.js \
  --url "${BASE_URL}windows7-v1.img" \
  --chunk-size 1048576 \
  --count 32 \
  --concurrency 4 \
  --passes 2 \
  --random \
  --seed 12345
```

Pass 1 typically warms the CDN. Pass 2 should skew toward `X-Cache: Hit from cloudfront` if byte-range caching is configured correctly.

### Custom domain / “same-origin” notes

This module can optionally attach one or more `custom_domain_names` (CNAMEs) to the CloudFront distribution.

#### DNS

- **CNAME**: `images.example.com` → `<distribution_domain_name>`
- **Route53 alias (recommended)**: Alias `A/AAAA` to the CloudFront distribution domain

If you set `custom_domain_names`, you must also set `acm_certificate_arn` (an ACM certificate **in `us-east-1`**, as required by CloudFront).

#### CORS

- If your web app loads disk image bytes from a **different origin** (e.g. app at `https://example.com`, images at `https://images.example.com`), the browser will require **CORS**.
- Fetching with `Range` headers typically triggers **CORS preflight** (`OPTIONS`) requests.
- Configure `cors_allowed_origins` accordingly.

If you allow **multiple** origins and rely on **S3** to emit CORS headers (`enable_edge_cors = false`), be aware that S3 will echo `Access-Control-Allow-Origin` based on the incoming `Origin` header, and CloudFront may cache that header along with the object. For multi-origin setups, prefer `enable_edge_cors = true` (and optionally `enable_edge_cors_preflight = true`) so CloudFront can add consistent CORS headers at the edge without fragmenting the cache.

If you truly need *same-origin* (same scheme/host/port as your app), you usually need to serve both your app and `/images/*` through the **same CloudFront distribution**. You can still use this module as a reference for the S3 + OAC + caching pieces.

##### Edge-handled preflight (`OPTIONS`) for `/images/*` (optional)

When `enable_edge_cors_preflight = true`, a CloudFront Function responds to CORS preflight requests for `/images/*` at the edge (viewer request), reducing origin load and latency for the first `Range` fetch.

Behavior summary:

- Only handles CORS preflight requests (requests with `Origin` and `Access-Control-Request-Method`).
- Validates `Origin` against `cors_allowed_origins` (exact origin match like `https://app.example.com`, or `*`).
- Returns `204` and includes:
  - `Access-Control-Allow-Origin: <origin>` (or `*` when configured with a wildcard allowlist and no credentials)
  - `Access-Control-Allow-Methods: GET,HEAD,OPTIONS`
  - `Access-Control-Allow-Headers: ...` (from `cors_allowed_headers`, must include `Range`)
  - `Access-Control-Allow-Credentials: true` (only when `cors_allow_credentials = true`)
  - `Cross-Origin-Resource-Policy: <value>` (only when `cross_origin_resource_policy` is set)
  - `Access-Control-Max-Age: <seconds>` (from `cors_max_age_seconds`)
  - `Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers`
- Non-CORS `OPTIONS` requests to `/images/*` return `404` (to avoid forwarding `OPTIONS` to S3).

Example:

```bash
curl -i -X OPTIONS "https://$CLOUDFRONT_DOMAIN/images/example.bin" \
  -H "Origin: https://app.example.com" \
  -H "Access-Control-Request-Method: GET" \
  -H "Access-Control-Request-Headers: Range"
```

Expected response headers (abridged):

```
HTTP/2 204
access-control-allow-origin: https://app.example.com
access-control-allow-methods: GET,HEAD,OPTIONS
access-control-allow-headers: Range,If-Range,Content-Type,If-None-Match,If-Modified-Since
access-control-max-age: 86400
vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

---

### Purged tree document: infra/local-object-store/README.md

## Local object store (MinIO) for Range + CORS testing

This directory provides a self-contained local environment to validate:

- **HTTP Range** behavior (`206 Partial Content`, `Content-Range`, etc)
- **CORS / preflight** behavior for `Range` requests (browser `OPTIONS` flow)
- (Optional) **CDN/proxy “edge” behavior** (CORS header overrides, preflight handling)

It is intended for local development workflows where disk images (multi‑GB) are stored in an S3-compatible object store.

### Services

| Service | Purpose | URL |
| --- | --- | --- |
| MinIO (S3 API) | S3-compatible origin | `http://localhost:9000` |
| MinIO Console | Upload/browse objects via UI | `http://localhost:9001` |
| `minio-proxy` (optional) | Reverse proxy in front of MinIO (edge/CDN emulation) | `http://localhost:9002` |

Default credentials:

- `MINIO_ROOT_USER=minioadmin`
- `MINIO_ROOT_PASSWORD=minioadmin`

Default bucket:

- `BUCKET_NAME=disk-images`

Default CORS origin:

- `CORS_ALLOWED_ORIGIN=http://localhost:5173` (Vite default)

Default CORP policy (proxy only):

- `CROSS_ORIGIN_RESOURCE_POLICY=cross-origin`

### Start / stop

From this directory:

```bash
docker compose up
```

From the repo root (using `just`):

```bash
just object-store-up
```

You can override defaults by exporting environment variables before starting (or by creating a `.env` file in this directory), for example:

```bash
export CORS_ALLOWED_ORIGIN=http://localhost:3000
docker compose up
```

There is an `.env.example` in this directory that you can copy to `.env` to get started.

If you change `MINIO_ROOT_USER` / `MINIO_ROOT_PASSWORD`, also reset volumes so the persisted `mc` config is regenerated:

```bash
docker compose --profile proxy down -v
```

Stop and remove containers (keeps volumes by default):

```bash
docker compose --profile proxy down
```

To also remove persisted data:

```bash
docker compose --profile proxy down -v
```

#### Optional proxy (“CDN”) layer

Enable the proxy container with a Compose profile:

```bash
docker compose --profile proxy up
```

Or from the repo root:

```bash
just object-store-up-proxy
```

The proxy is useful for reproducing “edge” behaviors (for example, overriding CORS headers and handling preflights at the proxy instead of the origin).

It also injects a `Cross-Origin-Resource-Policy` (CORP) header (configurable via `CROSS_ORIGIN_RESOURCE_POLICY`) to make it easier to test disk streaming under `Cross-Origin-Embedder-Policy: require-corp`.

For caching correctness, the proxy also sets:

- `Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers`

### Upload a large file

#### Option A: MinIO Console UI (no extra tooling)

1. Open `http://localhost:9001`
2. Log in with the credentials above
3. Open the `disk-images` bucket
4. Upload a file (for example, a multi‑GB disk image)

#### Option B: Use `mc` via Docker Compose (no local install)

Create a large file:

```bash
dd if=/dev/zero of=./large.bin bs=1M count=64
```

Upload it to MinIO:

```bash
docker compose --profile tools run --rm mc cp ./large.bin local/disk-images/large.bin
```

List objects:

```bash
docker compose --profile tools run --rm mc ls local/disk-images
```

### Verify Range responses (206 Partial Content)

> These examples assume you uploaded `large.bin` to `disk-images/large.bin`.

#### Automated smoke check (origin + proxy)

This directory includes a small script that will:

- start the containers
- upload a small random file
- (by default) upload it to `disk-images/_smoke/range-test.bin` (overwriting on each run)
- verify `HEAD`, `206` Range responses, and preflight behavior against both the origin and the proxy

```bash
bash ./verify.sh
```

Or from the repo root:

```bash
just object-store-verify
```

To stop containers at the end:

```bash
bash ./verify.sh --down
```

#### Verify HEAD / size discovery

The streaming disk client typically starts with a `HEAD` request to discover the object size (`Content-Length`).

```bash
curl -s -D - -o /dev/null -I \
  http://localhost:9000/disk-images/large.bin
```

Look for:

- `HTTP/1.1 200 OK`
- `Content-Length: ...`
- `Accept-Ranges: bytes`

#### Direct to MinIO origin

```bash
curl -s -D - -o /dev/null \
  -H 'Range: bytes=0-15' \
  http://localhost:9000/disk-images/large.bin
```

To validate the **CORS + ExposeHeaders** behavior, include an `Origin` header and look for `Access-Control-Expose-Headers: ... Content-Range ...`:

```bash
curl -s -D - -o /dev/null \
  -H 'Origin: http://localhost:5173' \
  -H 'Range: bytes=0-15' \
  http://localhost:9000/disk-images/large.bin
```

Expected:

- `HTTP/1.1 206 Partial Content`
- `Content-Range: bytes 0-15/<full-size>`

#### Via proxy (optional)

Start the proxy:

```bash
docker compose --profile proxy up
```

Then:

```bash
curl -s -D - -o /dev/null \
  -H 'Range: bytes=0-15' \
  http://localhost:9002/disk-images/large.bin
```

### Reproduce browser preflight behavior (CORS + Range)

### Benchmark Range throughput (optional)

This repo also includes a small Node-based Range harness for benchmarking chunked reads:

```bash
# Direct to MinIO origin:
node tools/range-harness/index.js \
  --url "http://localhost:9000/disk-images/large.bin" \
  --chunk-size 1048576 --count 32 --concurrency 4 --random

# Via the optional proxy (“edge”):
node tools/range-harness/index.js \
  --url "http://localhost:9002/disk-images/large.bin" \
  --chunk-size 1048576 --count 32 --concurrency 4 --random
```

Note: MinIO/Nginx may not emit a CDN-style `X-Cache` header; in that case the harness still provides latency/throughput metrics.

Browsers typically preflight a CORS request when you send a non-simple header like `Range`.

#### Browser console snippet (shows actual preflight)

From a page served at your configured origin (default `http://localhost:5173`), run:

```js
const url = "http://localhost:9002/disk-images/large.bin"; // proxy (recommended)
// const url = "http://localhost:9000/disk-images/large.bin"; // origin

const res = await fetch(url, { headers: { Range: "bytes=0-15" } });
console.log("status", res.status);
console.log("content-range", res.headers.get("content-range"));
console.log("bytes", new Uint8Array(await res.arrayBuffer()));
```

In DevTools → Network you should see an `OPTIONS` preflight followed by a `GET`, and `content-range` should be readable in JS.

#### Preflight against MinIO origin

```bash
curl -i -X OPTIONS \
  -H 'Origin: http://localhost:5173' \
  -H 'Access-Control-Request-Method: GET' \
  -H 'Access-Control-Request-Headers: range, if-range, if-none-match, if-modified-since' \
  http://localhost:9000/disk-images/large.bin
```

#### Preflight against the proxy (optional)

```bash
curl -i -X OPTIONS \
  -H 'Origin: http://localhost:5173' \
  -H 'Access-Control-Request-Method: GET' \
  -H 'Access-Control-Request-Headers: range, if-range, if-none-match, if-modified-since' \
  http://localhost:9002/disk-images/large.bin
```

For actual `GET` responses, ensure you can see (and access from JS) these headers:

- `Access-Control-Allow-Origin` (matches your app origin)
- `Access-Control-Expose-Headers: ... Content-Range ...`

### Notes: MinIO vs AWS S3

- **Addressing style:** MinIO commonly uses *path-style* URLs (`/bucket/key`). AWS S3 increasingly prefers *virtual-hosted-style* (`bucket.s3.amazonaws.com/key`).
- **CORS configuration:** AWS S3 CORS is configured per-bucket. MinIO’s CORS behavior is configured at the API layer (this compose setup wires `CORS_ALLOWED_ORIGIN` into `MINIO_API_CORS_ALLOW_ORIGIN`).
- **Auth:** This compose setup makes the bucket **public-read** (`mc anonymous set download`) so that browser/curl tests don’t require request signing. Production buckets should generally require auth and/or be fronted by a CDN.
- **Proxy/CDN behavior:** Real CDNs (e.g. CloudFront) can:
  - Handle/terminate `OPTIONS` at the edge
  - Add/remove CORS headers
  - Cache (and sometimes break) `Range` responses depending on configuration

### Publish a chunked disk image (no HTTP Range)

This repo also supports serving disk images without HTTP `Range` by publishing the image as many fixed-size chunk objects plus a `manifest.json` (see [`../specs/chunked-disk-image-format.md`](../specs/chunked-disk-image-format.md)).

#### Publish with `aero-image-chunker`

From the repo root:

```bash
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin

# Create a sample file (5 MiB).
dd if=/dev/urandom of=./scratch.img bs=1M count=5

# Default chunk size is 4 MiB (4194304). Pass --chunk-size to override.
cargo run --locked --manifest-path tools/image-chunker/Cargo.toml -- publish \
  --file ./scratch.img \
  --bucket disk-images \
  --prefix images/demo/sha256-test/ \
  --endpoint http://localhost:9000 \
  --force-path-style \
  --region us-east-1 \
  --concurrency 4
```

Then verify the published manifest + chunks end-to-end:

```bash
cargo run --locked --manifest-path tools/image-chunker/Cargo.toml -- verify \
  --bucket disk-images \
  --prefix images/demo/sha256-test/ \
  --endpoint http://localhost:9000 \
  --force-path-style \
  --region us-east-1 \
  --concurrency 4
```

#### Verify with `curl`

This compose setup makes the bucket anonymously readable, so you can verify without signing:

```bash
curl -fSs http://localhost:9000/disk-images/images/demo/sha256-test/manifest.json | head
curl -fSsI http://localhost:9000/disk-images/images/demo/sha256-test/chunks/00000000.bin
```

---

### Purged tree document: test-images/README.md

## Test images (local-only)

This directory is intentionally **gitignored** (except this README).

It is used for large binaries that we cannot/should not commit:

- Open-source OS media downloaded by scripts (e.g. FreeDOS).
- User-supplied proprietary media (e.g. Windows 7).

Small, deterministic fixtures that *are* committed to the repo (e.g. tiny boot
sectors used by integration tests) live under `tests/fixtures/`.

### Prepare open-source images

```bash
bash ./scripts/prepare-freedos.sh
```

This writes `test-images/freedos/fd14-boot-aero.img` which is a FreeDOS 1.4 boot
floppy patched to print `AERO_FREEDOS_OK` to `COM1` during startup.

### Windows images (local only)

Windows images must be provided by the developer and **must not be committed**.
See:

```bash
bash ./scripts/prepare-windows7.sh
```

If you need to prepare a Windows 7 SP1 install ISO to load Aero drivers/certs during setup and first boot, see:

- `../areas/windows-drivers.md`

---
