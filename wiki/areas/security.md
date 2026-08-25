# Security

> Aero's security model: the trust boundaries between guest, host page, and
> proxy; the browser isolation contract; the egress policy that keeps the guest
> from using the proxy as an open relay; and the input-validation rules that
> apply to every protocol decoder in the tree.

This document complements [`SECURITY.md`](../history/purged-material.md). It focuses on **everyday security hygiene** for contributors: handling secrets, responding to leaks, and triaging automated security findings.

## Secrets (local development)

### `.env` files

- **Never commit real secrets** (API keys, JWT signing keys, OAuth secrets, database URLs with passwords, etc).
- Use `*.env.example` files as **templates** with placeholder values.
  - Copy: `cp .env.example .env` (or `cp path/to/.env.example path/to/.env`)
  - Fill in values locally.
- `.env` files are gitignored by default.
  - If you need a new environment variable, update the relevant `*.env.example` file and any docs/scripts that reference it.

### Other common secret files

Avoid committing credentials in any form, including:

- `.npmrc` (can contain registry tokens)
- `.netrc` (contains machine credentials)
- `*.pem` / `*.key` / `*.p12` / `*.pfx` (private keys / cert bundles)
- `terraform.tfvars` / `*.tfvars` (frequently contains credentials)

If you genuinely need a certificate/key for tests, it must be a **non-sensitive fixture** and should be clearly documented as such.

## If a secret is leaked

Treat any leak as compromised.

1. **Rotate immediately**
   - Revoke/rotate the token, password, or key in the upstream system (GitHub, cloud provider, IdP, etc).
   - Assume the secret has been harvested if it was pushed to a public branch.
2. **Assess blast radius**
   - Identify what the secret could access (scopes/permissions).
   - Review audit logs if available.
3. **Remove it from history (optional but recommended)**
   - Rotation is the priority; history rewrite is secondary.
   - If required, use `git filter-repo` (preferred) or BFG to purge the secret, then force-push to affected branches.
4. **Notify maintainers privately**
   - Use the reporting process in [`SECURITY.md`](../history/purged-material.md).
   - Include: what leaked, when, what was rotated, and any follow-up actions.

## Automated scanning (CodeQL + secret scanning)

### CodeQL (static analysis)

CodeQL **is not run.** The workflow that ran it was purged with the rest of CI.
The configuration it used covered:

- Rust (workspace)
- JavaScript/TypeScript (repo sources; dependencies installed from the canonical Node workspace)
- Go (`proxy/webrtc-udp-relay`)

Runs:

- weekly on a schedule
- on pull requests that touch Rust/TS/Go paths

Results are uploaded to **GitHub → Security → Code scanning alerts**.

Query selection and exclusions lived in a CodeQL config alongside it, using `security-and-quality` and excludes low-precision queries to keep initial alert noise manageable.

### Triage process

When an alert is filed:

1. **Reproduce/understand** the finding (read the CodeQL query help and the trace).
2. Prefer **fixing** the issue in code.
3. If it is a false positive / acceptable risk:
   - **Dismiss** the alert in GitHub with a clear justification and (ideally) link to an issue.
   - Or add a **targeted suppression** with justification:
      - Prefer inline suppressions close to the code (so the rationale lives with the code).
      - Use the `codeql[...]` suppression comment format supported by the language, e.g.
        - `// codeql[javascript/<rule-id>] <justification>`
        - `// codeql[rust/<rule-id>] <justification>`
        - `// codeql[go/<rule-id>] <justification>`

Avoid broad suppressions (like disabling an entire query suite) unless there is a documented, reviewed reason.

### Secret scanning

If GitHub Secret Scanning is enabled for this repo, treat any alert as high priority:

- rotate/revoke first
- then clean up the repo/history as needed
- document what happened so we don’t repeat it

## Security headers (CSP, COOP/COEP, Permissions-Policy)

Aero needs a stricter-than-usual set of browser capabilities:

- **WebAssembly threads / `SharedArrayBuffer`** → requires **cross-origin isolation** (COOP/COEP).  
  See [Cross-origin isolation](../decisions/0002-cross-origin-isolation.md) and [WebAssembly build variants](../decisions/0004-wasm-build-variants.md) for how this ties into threaded vs single-threaded builds.
- **Dynamic WebAssembly compilation** for JIT blocks (e.g. `WebAssembly.compile`, `WebAssembly.instantiate`, `WebAssembly.compileStreaming`) → requires CSP **`'wasm-unsafe-eval'`**.
- **Web Workers / module workers** (and potentially bundler-generated `blob:` workers) → requires CSP **`worker-src 'self' blob:`**.
- **WebGPU** + **OPFS** do not require CSP directives, but CSP should avoid accidentally blocking the resources needed to start the app (scripts, workers, WASM fetches, WebSocket proxy).

This document defines a **secure-by-default** header set and provides templates for common hosting providers.

---

### Recommended header set

#### Cross-origin isolation (required for threads)

These are required for `SharedArrayBuffer` (and therefore `wasm32-threads` / parallel workers) in modern browsers:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp`
- `Cross-Origin-Resource-Policy: same-origin`
- `Origin-Agent-Cluster: ?1` (recommended hardening; keeps the origin in an origin-keyed agent cluster)

**Tradeoff:** COEP will block embedding cross-origin resources unless they send `Cross-Origin-Resource-Policy` / CORS headers. Keep all JS/WASM/assets same-origin where possible.

#### Content Security Policy (CSP)

Recommended CSP (single line):

```
default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; connect-src 'self' https://aero-gateway.invalid wss://aero-gateway.invalid; img-src 'self' data: blob:; style-src 'self'; font-src 'self'
```

Directive rationale:

- `default-src 'none'`: deny by default; explicitly allow what Aero needs.
- `script-src 'self' 'wasm-unsafe-eval'`: allow ESM from same-origin **and** dynamic WASM compilation for JIT **without** enabling JS `eval`.
- `worker-src 'self' blob:`: allow module workers from same-origin; allow `blob:` workers for bundlers/worklets that generate worker code at runtime.
- `connect-src 'self' …`: allow `fetch()` / `WebAssembly.compileStreaming()` from same-origin and optionally a WebSocket proxy origin.
  - `https://aero-gateway.invalid` and `wss://aero-gateway.invalid` are **documentation-only placeholders** (the `.invalid` TLD will never resolve). Replace with your real gateway/proxy origin or remove them entirely.
- `img-src 'self' data: blob:`: allow icons and generated object URLs.
- `style-src 'self'`: allow same-origin CSS without allowing inline script execution.
  - If you must use inline styles (e.g. CSS-in-JS), consider `'unsafe-inline'` here **only** (avoid it in `script-src`).
- `base-uri 'none'`, `object-src 'none'`, `frame-ancestors 'none'`: reduce common injection and clickjacking risks.

##### No inline scripts/styles by default

This CSP intentionally does **not** include `'unsafe-inline'`. That means:

- `<script>…</script>` (inline) will be blocked
- `<style>…</style>` and `style="…"` (inline) will be blocked

Prefer external JS/CSS files loaded from `'self'`. If you absolutely must use inline code, use **nonces/hashes** rather than `'unsafe-inline'` (note that many static host header configs cannot inject per-request nonces).

##### Why `wasm-unsafe-eval` is needed

Aero’s JIT design compiles new WASM modules at runtime (e.g. for hot x86 blocks). Under CSP, WASM compilation is controlled by the `script-src` directive:

- Without `script-src 'wasm-unsafe-eval'`, browsers may block:
  - `WebAssembly.compile(...)`
  - `WebAssembly.instantiate(...)` when given raw bytes
  - Streaming variants that compile from a response

`'wasm-unsafe-eval'` is preferred over `'unsafe-eval'` because it enables WASM compilation while still blocking classic JavaScript eval sinks like `eval()` and `new Function()`.

##### Browser support notes

`'wasm-unsafe-eval'` is the modern, least-bad way to permit dynamic WASM compilation. If a target browser does not recognize it, you have two options:

- **Disable runtime compilation** (ship precompiled modules only; no JIT tier), or
- As a last resort, add **`'unsafe-eval'`** (significantly weaker; enables JS `eval`/`new Function`).

##### Risk tradeoffs of `wasm-unsafe-eval`

Enabling dynamic WASM compilation:

- **Expands the set of executable code sources** (code can be created from bytes at runtime).
- Makes certain classes of “code as data” bugs more dangerous (e.g. if untrusted input can reach a WASM compiler path).

However:

- It is **much narrower** than `'unsafe-eval'` (no JS `eval`).
- WASM still executes inside the browser sandbox; it does not grant native code execution.

If you can avoid runtime compilation (e.g. ship precompiled WASM modules only), you can remove `'wasm-unsafe-eval'` for an even tighter policy, but Aero’s JIT tier may not work.

#### Tightening `connect-src` (recommended)

Keep `connect-src` as narrow as possible because it governs:

- `fetch()` and `WebAssembly.compileStreaming()` network loads
- `WebSocket` / `WebRTC` signaling (if used)

Recommendations:

- If the proxy can be hosted **same-origin** (e.g. behind the same domain), use:
  - `connect-src 'self'`
- If the proxy is on a separate origin, add the **exact** origin(s) only:
  - `connect-src 'self' https://proxy.example.com wss://proxy.example.com`

#### Other security headers

Recommended baseline:

- `Referrer-Policy: no-referrer` (privacy-first; alternatively `strict-origin-when-cross-origin`)
- `X-Content-Type-Options: nosniff`
- `Permissions-Policy: camera=(), geolocation=(), microphone=(self), usb=(self)`

Notes:

- WebUSB is controlled by the Permissions-Policy **`usb`** directive. If WebUSB is disabled via policy, calls like `navigator.usb.requestDevice()` will throw `SecurityError` even if `navigator.usb` exists.
  - Allow WebUSB on the top-level (same-origin) only:
    - `Permissions-Policy: camera=(), microphone=(), geolocation=(), usb=(self)`
  - Disable WebUSB entirely:
    - `Permissions-Policy: camera=(), microphone=(), geolocation=(), usb=()`
  - Iframe delegation: to use WebUSB in an iframe, the embedding page must include `allow="usb"` on the `<iframe>`, and the iframe's origin must be permitted by the embedding document's Permissions-Policy.
- If you do not need microphone capture, you can disable it with `microphone=()`. Aero’s web UI uses the microphone only when the user explicitly enables it.

---

### Where the headers actually come from

The intended single source is **`scripts/headers.json`** with
`scripts/security_headers.mjs` applying it. The browser side does import from
there. **The gateway does not** — `crossOriginIsolation.ts` and
`securityHeaders.ts` hardcode their own literals and have already drifted from
the shared set. Treat "one source" as the goal, not the current state:

- `vite.harness.config.ts` and `apps/web/vite.config.ts` for dev and preview.
- `services/gateway/src/middleware/crossOriginIsolation.ts` and
  `securityHeaders.ts` when the gateway is the origin.
- `apps/web/public/_headers`, copied into the build output — the one surviving
  static-host template, and the reason a `_headers`-aware host works without
  further configuration.

One source with several consumers is what *would* stop the browser and gateway
sides drifting apart. Until the gateway imports it, the two can disagree — and a
check comparing the served headers against `scripts/headers.json` on both paths
is the missing gate.

> The hosting-provider and reverse-proxy templates that used to be listed here —
> Netlify, Vercel, Cloudflare Pages, Caddy, nginx, and a Helm chart — described
> files that were purged, along with the CI job that validated them. Deployment
> is out of scope and on the ban list; see
> [the working agreements](../meta/working-agreements.md).


### Verification checklist

In a production build, open DevTools → Console and ensure:

- No CSP violations on startup.
- WASM loads and initializes normally.
- A synthetic dynamic compilation works (example):

```js
await WebAssembly.compile(new Uint8Array([0x00,0x61,0x73,0x6d,0x01,0x00,0x00,0x00]));
```

If you see errors like `Refused to compile or instantiate WebAssembly module because 'wasm-unsafe-eval' is not an allowed source`, your deployed CSP is missing `'wasm-unsafe-eval'` in `script-src`.

#### Repo-local CSP/COOP/COEP regression tests

This repo includes an automated browser PoC and Playwright coverage to prevent regressions:

- CSP/COOP/COEP PoC app: `apps/web/public/wasm-jit-csp/`
- CSP test server (sets COOP/COEP + CSP variants): `tests/helpers/csp_server.mjs`
- Playwright spec: `tests/e2e/csp-fallback.spec.ts`
- Playwright spec (preview server headers): `tests/e2e/security-headers.spec.ts`

Run locally:

```bash
node tests/helpers/csp_server.mjs --port 4180
# then open:
#  - http://127.0.0.1:4180/csp/strict/
#  - http://127.0.0.1:4180/csp/wasm-unsafe-eval/
```

Or run the automated check:

```bash
pnpm exec playwright test tests/e2e/csp-fallback.spec.ts
```

And validate the canonical preview server header set:

```bash
pnpm run test:security-headers
```

The host-side capability bit that gates Tier-1/2 WASM JIT is exposed as:

- `jit_dynamic_wasm` in `src/platform/features.ts` and `apps/web/src/platform/features.ts`
