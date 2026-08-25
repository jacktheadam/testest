# Retirements

> The graveyard index: what was removed from the tree, why, and where its
> content lives now. Every removal is recorded here in the same change that
> removes it, so a future reader can always answer "what happened to X?".
>
> Verdicts follow the cleanup doctrine in
> [../meta/working-agreements.md](../meta/working-agreements.md). Aspirational plans, release
> ceremony, deployment production-isation, and foundation-era legal and
> governance writing are project theater and are purged; enforceable policy and
> live contracts are absorbed into the wiki instead.
>
> Verdict legend: **RETIRE** means nothing was carried beyond this record.
> **ABSORB** means the durable facts moved into the wiki page named alongside.
>
> Where living truth already exists elsewhere — the repo state page, the
> decisions, or the code — this page points rather than duplicating.

## The documentation corpus

The `docs/` tree, the per-directory READMEs, the sprint-era workstream briefs,
and the documentation carried by the purged subtrees have all been absorbed into
the wiki. That absorption is *lossless by construction*: reference material was
re-homed and merged rather than summarised, and the raw originals of everything
are kept on disk in a gitignored `.attic/` — removed from the project, not
destroyed.

What each source became is recorded in the sections below. Three collection
pages exist for material that has no live home but is worth keeping readable:

- [sprint-era-record.md](./sprint-era-record.md) — the eight-day sprint's
  milestone plan, workstream briefs, and legal analysis.
- [purged-material.md](./purged-material.md) — governance, legal, and policy
  templates for a public project Aero is not, plus the READMEs of removed
  subtrees.
- [retired-graphics-prototypes.md](./retired-graphics-prototypes.md) — the GPU
  driver strategy debate and the command protocols that lost it.

## The theater and legacy purge — RETIRE

This is the consolidated deletion record for the cleanup that removed project
theater and the legacy parallel stacks. The doctrine that authorised it, and the
ban list that keeps it from coming back, are in
[../meta/working-agreements.md](../meta/working-agreements.md); this section is
the inventory of what actually went.

**Continuous-integration and hosting apparatus.** The workflow directory in full
— 46 workflows plus composite actions, code-scanning config and the dependency
bot — along with `deploy/`, `infra/`, `.devcontainer/`, both competing compose
files, `.dockerignore`, `codecov.yml`, `netlify.toml`, `vercel.json` and
`.markdownlint-cli2.jsonc`. The CI-coupled helpers went with them: the
pull-request URL script and its package scripts, eleven CI-infrastructure
contract tests, and five check scripts that only existed to validate deployment
manifests, infrastructure-as-code, security headers, and the workflow files
themselves. `check-toolchains.mjs`, the `justfile` and `check-repo-layout.sh`
were trimmed of the sections that referenced any of it. Verification here is
run-by-agent; the gates are commands anyone can run.

**Project-theater and legal files.** `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`,
`SECURITY.md`, `LEGAL.md`, `TRADEMARKS.md`, `DMCA_POLICY.md`,
`TERMS_OF_SERVICE_TEMPLATE.md`, `PRIVACY_POLICY_TEMPLATE.md`, `AUTHORS`,
`NOTICE`, and **every licence file** — the root `LICENSE-MIT` and
`LICENSE-APACHE`, the per-subproject copies under the virtio driver common tree
and the WebRTC relay, and both `THIRD_PARTY_NOTICES.md` (with the packaging
scripts patched to match; no binaries are actually redistributed in-tree). The
texts themselves are preserved in [purged-material.md](./purged-material.md).

**Legacy parallel stacks, after absorbing what was unique.** Relocated first:
the disk gateway to `tools/disk-gateway`, the L2 tunnel prototype to
`tests/helpers/`, and the Windows 7 unattend media to `guest-tools/unattend/`.
Then purged: the rest of `server/` (self-declared legacy), `tools/aero-gateway-rs`
(superseded by the gateway service and the L2 proxy), `windows-drivers/` (a stale
duplicate generation whose flat files were superseded by the organised driver
tree), `js/` (a re-export indirection whose canonical shims live per-crate),
`poc/`, `prototype/`, `guest/` (tombstones), the `images/` and `test-images/`
placeholders, the emptied `windows/` shell, and `instructions/`.

**`REFACTOR.md`** — a 505-line sprint driving prompt and 61-phase hardening log.
A process artefact whose durable outcomes are in the merged commits and in this
wiki.

Two things are worth knowing about the shape of this purge. The deletions are
**deliberately uncommitted** — the working tree carries them while the index
still lists the files — which is why `check-repo-layout.sh` grew a guard for
unstaged deletions and why the hygiene test filters `git ls-files` through an
existence check. And nothing was destroyed: the originals are in a gitignored
`.attic/`.

## Project milestones & roadmap — RETIRE

`project-history.md`

An 18-month, six-phase development plan (Foundation → Core Emulation →
Graphics & I/O → Win7 Compatibility → Performance → Production Release) with
weekly milestone tables, exit criteria, MIPS/FPS/app-compat targets, team-size
estimates, and a 2024–2025 timeline.

Retired because it is aspirational planning theater: the timeline predates the
repo's actual existence (the tree was written in an 8-day sprint, 2026-01-10 →
2026-01-17 — see `wiki/state/repo-state-and-structure.md` §8.1), none of the
metrics was ever measured, and the state doc already flags this class of
milestone table as *aspirational planning, not measured reality* (state doc
§2). The real, measured milestone ladder lives in `wiki/areas/testing.md`. No
facts preserved.

## Agent task breakdown — RETIRE

`project-history.md`

The sprint-era work decomposition: ~250 tasks across core/graphics/I/O/
firmware/perf/infra/service lanes, each with a coded ID, priority, dependency,
and complexity, plus status-tracking templates and parallel-lane diagrams.

Retired because coded task identifiers are banned as primary names (house
rules, `wiki/README.md`), the tables were never maintained as ground truth —
the doc's own firmware section admits it is "primarily a historical breakdown
and **not** a reliable 'what's missing' list" — and work coordination now
lives in the wiki. Preserved facts (all verified in-tree):

- The interface anchors it cites are real: the CPU bus trait `CpuBus`
  (`crates/aero-cpu-core/src/mem.rs:61`), the VGA display trait
  `DisplayOutput` (`crates/aero-gpu-vga/src/lib.rs:188`), and the host-side
  GPU command processor `AeroGpuCommandProcessor`
  (`crates/aero-gpu/src/command_processor.rs:388`).
- Its canonical-virtio claim holds: the device-model virtio implementation is
  `crates/aero-virtio`, used by `crates/aero-machine`; the legacy
  emulator-local virtio stack is gone.
- Its Windows-driver contract pointers are real: the definitive Win7 virtio
  contract `wiki/areas/windows-drivers.md` (`AERO-W7-VIRTIO`) and
  machine manifest `protocol-vectors/windows-device-contract.json` (both now
  relocated out of `docs/`).

## Legal & licensing considerations — RETIRE

`project-history.md`

A long-form emulation-legality survey written for a public foundation:
case law (Sony v. Connectix, Sega v. Accolade), Windows 7 EULA analysis,
patent and codec tables, trademark naming rules, DMCA safe-harbor procedure,
international jurisdictions, compliance checklists, and risk matrices.

Retired because it is governance/legal theater for a repo that is agent-owned
and distributes nothing (the cleanup doctrine and ban list in the working agreements). Its
self-declared "authoritative policies" — `LEGAL.md`, `TRADEMARKS.md`,
`DMCA_POLICY.md`, `SECURITY.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` —
were all purged in the 2026-07-23 theater purge, so the doc's own frame of
reference is gone. Preserved facts (the two operational rules that still bind
this repo):

- **Clean-room rule**: never copy or redistribute Microsoft proprietary code
  (Windows, BIOS, drivers); Microsoft driver *samples* are usable only when
  OSI-permissively licensed with notices preserved. The live form of this
  policy is `drivers/windows7/LEGAL.md`.
- **No proprietary media in-repo**: users supply their own Windows media and
  license; nothing copyrighted ships in the repository. This is still
  mechanically enforced — see the fixtures-policy section below.

## Documentation license — RETIRE

`project-history.md`

Declared `docs/` content CC-BY-4.0 (with code MIT OR Apache-2.0). Retired
because all license files were purged by user decision (2026-07-22, recorded
recorded in the state page's history), and with `docs/` itself purged the declaration
has no object. No facts preserved beyond this record.

## Fixtures & test-assets policy — ABSORB

`../areas/testing.md`

The repo's binary-fixture policy. Unlike the rest of this page its rules are
**live and enforced**, so they are absorbed in full (condensed):

- **Forbidden in-repo**: OS install media, disk/VM images, BIOS/firmware
  dumps, and Windows binaries — extensions including `.iso`, `.img`, `.vhd`,
  `.vhdx`, `.vmdk`, `.qcow`, `.qcow2`, `.wim`, `.exe`, `.dll`, plus anything
  under `*/test_images/windows*` or `*/fixtures/windows*`.
- **Enforcement**: `scripts/ci/check-repo-policy.sh` (run by agents directly;
  the PR/push CI framing in the source doc is dead). It applies a default
  20 MB blob cap (`SIZE_LIMIT_MB`), a stricter 1 MiB cap for allowlisted
  firmware-like blobs, an exact 64 KiB size check on `assets/bios.bin`, and
  git-blob-hash pins on the text placeholder drivers under
  `tools/packaging/aero_packager/testdata/` so they cannot drift into real
  proprietary binaries.
- **Allowlisted generated fixtures** (regenerate/check via
  `cargo xtask fixtures [--check]`): `assets/bios.bin` (via
  `cargo xtask bios-rom`),
  `crates/firmware/acpi/dsdt.aml` + `dsdt_pcie.aml` (clean-room `.asl`
  sources alongside), `tests/fixtures/boot/*.bin|.img`,
  `tests/fixtures/bootsector.bin`, `tests/fixtures/realmode_vbe_test.bin`,
  `tests/fixtures/boot/int_sanity.bin`, `tools/qemu_diff/boot/boot.bin`.
- **Golden images**: `tests/golden/` holds synthetic PNGs regenerated via
  `npm run generate:goldens`, drift-checked by `scripts/ci/check-goldens.mjs`.
- Dropped as stale: the pointer to a gitignored `test-images/` directory
  (purged 2026-07-23, cleanup phase 2) and the "CI runs on PRs" framing.

Note: this is operational policy, not history — if a future `wiki/meta/`
repo-policy page appears, this paragraph should move there.

## Release process — RETIRE

`../areas/build-and-tooling.md`

The release runbook: five GitHub Actions workflows (`release-web`,
`release-gateway`, `release-l2-proxy`, `release-webrtc-udp-relay`,
`release-storage-server`) publishing an `aero-web-<tag>.zip` static bundle and
four GHCR container images on semver tags, plus Netlify/Vercel/Cloudflare
Pages deploy recipes, docker-compose and Helm/k8s install snippets.

Retired because the entire surface it operates was purged 2026-07-23:
`.github/` (all 46 workflows), `deploy/`, `netlify.toml`, `vercel.json`
(cleanup phase 1; CI and production-ization configs are banned,
the ban list in the working agreements). There is no release train to document. Preserved
facts — the provenance surface it described is real code and survives:

- Web builds emit `aero.version.json` (version/gitSha/builtAt) from
  `vite.harness.config.ts:85` (and the legacy app config
  `web/vite.config.ts:107`).
- `GET /version` exists in `services/gateway` (`src/server.ts:303`),
  `crates/aero-l2-proxy` (`src/server.rs:183`), `crates/aero-storage-server`
  (`src/api/mod.rs:26`), and `proxy/webrtc-udp-relay`
  (`internal/httpserver/server.go`).
- The toolchain pins it cites (`.nvmrc`, `rust-toolchain.toml`,
  `scripts/toolchains.json`) remain the spec — already covered by the state
  doc §4, not duplicated here.

## Deployment & hosting (COOP/COEP) — RETIRE

`../areas/build-and-tooling.md`

The cross-origin-isolation deployment guide: mandatory COOP/COEP/CORP headers,
why they're needed (`SharedArrayBuffer`/WASM threads), hosting templates for
Netlify/Vercel/Cloudflare Pages/Caddy/nginx, GitHub-Pages unsuitability, IaC
CI validation, and an OPFS storage-quota/persistence section.

Retired because its durable kernel is already standing truth — cross-origin
isolation and the dual WASM variants are covered by the state doc §1 and
`wiki/decisions/0002-cross-origin-isolation.md` /
`0004-wasm-build-variants.md` — and everything else was production-ization:
`deploy/caddy`, `deploy/nginx`, `deploy/k8s`, the Pages/Netlify/Vercel
configs, and the IaC CI were all purged 2026-07-23. The source doc is
internally stale: its validated-files list already says "removed with the CI
purge, 2026-07-23" while its templates section still presents those same files
as provided. Preserved facts (verified):

- Canonical header values live in `scripts/headers.json` +
  `scripts/security_headers.mjs`, imported by `vite.harness.config.ts`; the
  same set is applied gateway-side in
  `services/gateway/src/middleware/crossOriginIsolation.ts` and
  `securityHeaders.ts`. The one surviving static-host template is
  `apps/web/public/_headers`.
- Header presence is testable via `npm run test:security-headers`
  (`tests/e2e/security-headers.spec.ts`); the non-isolated fallback path is
  testable via `VITE_DISABLE_COOP_COEP=1 npm run dev` (loads the
  single-threaded WASM variant).
- GitHub Pages remains unsuitable: it cannot send custom response headers, so
  cross-origin isolation is impossible there.
- The OPFS quota/persistence UI notes (storage panel,
  `navigator.storage.persist()`) belong to the storage-area curation, not
  history — flagged, not absorbed here.

## CloudFront disk streaming — RETIRE

`../areas/storage.md`

A complete AWS production recipe: private S3 + CloudFront with Origin Access
Control, `/public/*` vs `/users/*` cache behaviors, signed URLs vs signed
cookies, cache-key policy, CORS/CORP response-header policies, S3 CORS rules,
and a backend signing example.

Retired because it is production-ization for an imaginary deployment (CDN
configs are banned by the working agreements), and its companion reference
service `services/image-gateway/` is itself classified purge-pending (the
services-scope decision, recorded in the state page's history). Preserved facts — the
deployment-independent protocol constraints:

- Byte-range streaming requires a byte-stable wire representation: no
  compression/transforms on disk-byte routes (`Cache-Control: no-transform`,
  identity `Content-Encoding`); a `gzip` content-encoding makes byte offsets
  meaningless.
- Edge-validated authorization (signed cookies/URLs checked before cache
  lookup) makes private objects safe to cache without auth material in the
  cache key; conversely, origin-checked `Authorization` plus CDN caching is
  the classic private-data-leak footgun.
- Never vary the cache key by `Range`; align client reads to a fixed chunk
  size (default 1 MiB) for local-cache efficiency.

The normative streaming contract itself lives in
`../areas/storage.md` and `../areas/storage.md`
(absorbed by the storage-area curation; not duplicated here).

## Disk image streaming service runbook — RETIRE

`../areas/storage.md`

An operations runbook for a hosted disk-bytes service: the HTTP `Range`
contract, CORS/preflight requirements, ETag/caching policy, reverse-proxy and
CDN pitfalls, a `manifest.json` LocalFS catalog format, and security
hardening guidance.

Retired because it is hosted-service operations content that overlaps the
normative streaming-auth doc, and its named reference implementation
(`services/image-gateway/`) is purge-pending. Preserved facts (verified):

- The canonical client-side implementation is `aero_storage::StreamingDisk`
  (`crates/aero-storage`); the in-tree metadata/catalog server is
  `crates/aero-storage-server`, which treats image IDs as opaque,
  traversal-safe segments restricted to `[A-Za-z0-9._-]`
  (`crates/aero-storage-server/src/store/manifest.rs`, `store/mod.rs`).
- The durable browser contract: satisfiable `Range` → `206 Partial Content`
  with `Content-Range` + `Accept-Ranges: bytes` (`416` when unsatisfiable);
  `Range`/`If-Range` are not CORS-safelisted, so cross-origin reads preflight;
  `Content-Range`/`Accept-Ranges`/`Content-Length` must appear in
  `Access-Control-Expose-Headers` or the browser hides them from JS.

## Aero Gateway API + OpenAPI — ABSORB

`../specs/gateway-api.md`, `services/gateway/openapi.yaml`

The public contract of the networking backend (draft v1) plus its HTTP
schema. The gateway itself is canonical and kept (`services/gateway`),
so the contract remains real; the doc form is retired because the code plus
golden conformance vectors are the maintained source of truth (and a prose
spec this size would drift — it already partially duplicates
`../specs/auth-tokens.md` and `../specs/l2-tunnel-protocol.md`). Absorbed as a
contract summary, verified against the implementation:

- `POST /session` issues the opaque `aero_session` cookie:
  `<payload_b64url>.<sig_b64url>`, HMAC-SHA256 over the base64url payload
  *string*, signature exactly 43 chars, payload `{v: 1, sid, exp}` — expired
  when `exp * 1000 <= now`. Implemented in
  `services/gateway/src/session.ts`.
- `GET /tcp` (WebSocket upgrade, RFC6455 v13 handshake required) is a raw
  byte tunnel, one TCP connection per socket: `?v=1&host=&port=`, with a
  compatibility `target=host:port` alias that wins when both forms are
  present. Auth = the session cookie; missing/invalid → `401`.
- `GET /tcp-mux` multiplexes many TCP streams over one WebSocket with
  subprotocol `aero-tcp-mux-v1`: 9-byte big-endian frame header
  (`msg_type`, `stream_id`, `length`), message types OPEN/DATA/CLOSE/ERROR/
  PING/PONG. Codec/handler: `services/gateway/src/protocol/tcpMux.ts`,
  `src/routes/tcpMux.ts`; browser client: `apps/web/src/net/tcpMuxProxy.ts`.
- `/dns-query` is RFC 8484 DoH (GET `?dns=` and POST, both
  `application/dns-message`); `/dns-json` is a Cloudflare-schema-subset
  convenience endpoint (A/AAAA/CNAME). Both require the session cookie.
- Optional UDP-relay block in the session response plus
  `POST /udp-relay/token`: HS256 JWT (`sid`/`iat`/`exp`, optional `origin`),
  served only when the gateway is configured with a relay base URL. The relay
  itself is `proxy/webrtc-udp-relay` (protocol:
  `proxy/webrtc-udp-relay/PROTOCOL.md`).
- Security model: Origin allowlist, outbound port allowlist, optional
  hostname allow/deny lists, blocked private/special-purpose destination IP
  ranges (SSRF guard), per-session rate/connection/byte quotas.
- `/l2` is **not** served by the Node gateway — it is routed by the edge to
  `crates/aero-l2-proxy` (see `wiki/areas/networking.md`).
- Cross-language drift is pinned by golden vectors:
  `protocol-vectors/auth-tokens.json`, `protocol-vectors/tcp-mux-v1.json`,
  and `crates/conformance/test-vectors/aero-vectors-v1.json`. Local-dev
  relays implementing the same surface: `services/net-proxy/` (`/tcp`, `/tcp-mux`,
  `/udp`, `/dns-query`, `/dns-json`) and `tools/net-proxy-server/`.

Follow-up (out of scope for this page by assignment): this summary is a stub —
the full endpoint/limits detail belongs in a future `wiki/areas/` networking
or backends page.

## Guest tools README — ABSORB

`../areas/windows-drivers.md`

A short doc defining the Windows-side Guest Tools bundle (virtio-blk/net/snd/
input + AeroGPU drivers, PnP installer with boot-critical storage seeding,
optional userland utilities) and its device-contract rule. Absorbed in full
(condensed; all verified):

- Installer scripts must not hardcode PCI IDs, subsystem IDs, or service
  names. The definitive virtio contract is
  `wiki/areas/windows-drivers.md` (`AERO-W7-VIRTIO`); the
  machine-readable manifest is `protocol-vectors/windows-device-contract.json`, which
  must stay consistent with it.
- The installer-facing config `guest-tools/config/devices.cmd` is **generated
  from the manifest** (`scripts/generate-guest-tools-devices-cmd.py`;
  drift-check `scripts/ci/gen-guest-tools-devices-cmd.py --check`; full
  contract drift check `cargo run -p device-contract-validator --locked`,
  tool at `tools/device_contract_validator/`).
- `guest-tools/setup.cmd` pre-seeds
  `HKLM\SYSTEM\CurrentControlSet\Control\CriticalDeviceDatabase` so
  boot-critical virtio-blk binds during setup; contract drift → driver bind
  failure or boot bluescreen.

(The two contract docs were `docs/` residents; they now live at
`wiki/areas/windows-drivers.md` and `protocol-vectors/`.)

## Legacy GPU ABIs — RETIRE

`retirements.md`,
`retirements.md`

Two tombstones from earlier GPU bring-up generations: the surface-centric
prototype command protocol v0.1 (`CREATE_SURFACE`/`UPDATE_SURFACE`/`PRESENT`,
8-byte-aligned rings, small MMIO register block) whose device model was
already removed from the codebase, and a bare "retired" notice for an even
earlier experimental command ABI.

Retired because both already self-declare deprecated and their only function
was redirection. Preserved fact: the canonical AeroGPU ABI (PCI ID
`A3A0:0001`, decision 0005) is defined by the C headers in
`drivers/aerogpu/protocol/` (`aerogpu_pci.h`, `aerogpu_ring.h`,
`aerogpu_cmd.h`, README) with Rust/TypeScript mirrors in `crates/aero-protocol/`.
Nothing else carried.

## Repo layout (canonical vs legacy) — ABSORB

`../state/repo-state-and-structure.md`

The canon map: which trees are canonical/production and which are legacy,
quarantined, or prototype. Absorbed per assignment, minus what the state doc
already covers.

**Residual truths absorbed here** (verified; not present, or only thinly
present, in the state doc):

- **AeroGPU ABI wiring map** — the canonical Win7/WDDM graphics contract is
  the C headers in `drivers/aerogpu/protocol/` (source of truth), mirrored in
  `crates/aero-protocol/`; machine-side wiring lives in
  `crates/aero-machine/src/aerogpu.rs` (BAR0 registers + BAR1 VRAM/legacy
  decode for `A3A0:0001`), shared device-side implementation in
  `crates/aero-devices-gpu/`, and the legacy/sandbox integration surface in
  `crates/emulator/src/devices/pci/aerogpu.rs`. Full protocol mapping:
  `wiki/areas/graphics.md` (graphics area page).
- **Crate naming convention** — packages are `aero-foo` lowercase kebab-case
  in matching `crates/aero-foo/` directories; Rust `use` paths normalize
  `-` → `_`. Pre-convention crates without the prefix (e.g.
  `crates/emulator`, `crates/memory`) are grandfathered, but new crates must
  follow the convention (decision 0007).
- **USB canonical split** — device models + host controllers (UHCI/EHCI/xHCI)
  in `crates/aero-usb`; browser host integration + passthrough
  broker/executor in `apps/web/src/usb/` (decision 0015). Non-canonical: the
  `crates/emulator` `emulator::io::usb` integration shim. The older repo-root
  WebUSB demo RPC (`src/platform/legacy/webusb_*`) was already removed before
  the cleanup began.

**Already covered by `wiki/state/repo-state-and-structure.md` — not
duplicated here:**

- Canonical browser host = root `index.html` + `src/` (§5.1, §5.3);
  `apps/web/bringup.html` legacy/experimental (§5.1, §6 debt list).
- Rust workspace + `crates/aero-machine` as the single canonical VM
  integration layer (§5.2, §7 decisions 0008/0014); legacy crate stack
  classification (`crates/emulator`, `crates/devices`, `crates/memory`,
  `crates/platform`, `crates/firmware`, `crates/legacy/*`) per this very doc +
  decision 0001 (§5.2); migration debt `../areas/platform-and-firmware.md`
  (§6).
- Backend service rows: `services/gateway` canonical backend,
  `services/net-proxy/` dev relay, `proxy/webrtc-udp-relay`, `services/image-gateway`
  (§5.1; note the state doc says "active" while the cleanup plan marks
  `services/` purge-pending — the plan wins).
- `protocol-vectors/` purpose (§5.1); QEMU-based reference boot tests under
  root `tests/` + `crates/aero-boot-tests` (§2); ADR one-liners for 0001,
  0007, 0015 (§7).

**Stale in the source (recorded, not absorbed):** the doc's "quarantined
paths" section describes `server/` (+`server/LEGACY.md`),
`tools/aero-gateway-rs`, `poc/`, `prototype/`, and `guest/` as present — all
were purged 2026-07-23 (cleanup phases 1–2), so that section is now purely
this historical note.

---

## The development loop analysis — ABSORB

`../areas/debugging.md`, `../areas/testing.md`,
`../areas/platform-and-firmware.md`, `../areas/build-and-tooling.md`,
`../meta/engineering-principles.md`

A root-level working document that measured why the Windows 7 bring-up was slow
and proposed five changes. It was raw material rather than a wiki page: a dated
point-in-time analysis, unreferenced by any other page, whose status claims went
stale as the bring-up advanced. Its datapoints were verified against the tree and
absorbed; two did not survive verification and were corrected rather than
carried:

- *(Correction, found in a later review: an earlier version of this entry claimed
  the `just boot-*` targets were absent from the justfile. They are not — all five
  exist as thin wrappers over `scripts/win7.sh`. The source document was right and
  this record was wrong; both interfaces are now documented in
  `areas/debugging.md`.)*
- `aero-machine-cli` was said to pull 1,053 crates; the cone is **357**. The
  substance held — a boot-debugging binary still drags in `wgpu` and `naga` — so
  the corrected figure is in `areas/build-and-tooling.md`.

What was absorbed:

- The bring-up loop scripts and their real interfaces, the first-divergence
  method, and the `tools/tracediff/tracediff.py` rationale → `areas/debugging.md`.
- The kernel-debugging-over-COM1 contract and the full BCD investigation —
  approaches tried, the two controls that located the fault in the BCD edit
  rather than in Aero, and where to pick it up → `areas/debugging.md`.
- The boot ladder as a monotone metric → `areas/testing.md`.
- Adopting SeaBIOS as a way to delete a firmware variable → the open issues in
  `areas/platform-and-firmware.md`.
- The dependency weight of the bring-up binary → `areas/build-and-tooling.md`.
- Measure the loop before optimising it, and a green suite is not evidence about
  the goal → `meta/engineering-principles.md`.

Its stale measurements (a 15-minute cold build, a 16-minute boot, an interpreter
at 1.66 or 6.0 Minst/s, a project stuck behind `0xc0000225`) were deliberately
not carried: they describe a tree that no longer exists, and current interpreter
figures live in `areas/performance.md`.

## The Windows 7 bring-up strategy assessment — ABSORB

`../overview.md`, `../areas/testing.md`, `../areas/performance.md`,
`../areas/debugging.md`, `../meta/engineering-principles.md`

A root-level assessment of how far the project had got, whether the goal was
achievable, and what the development loop should look like. It was the clearest
case yet of raw material: a point-in-time judgement that had been amended in
place four times, so that later paragraphs contradicted earlier ones and the
header disclaimed the body. Its evidence was worth keeping; almost none of its
status claims were.

Verified and carried: 83 crates, ~1.1M lines of Rust, ~9,100 tests and 44 fuzz
targets all check out against the tree today, as does the QEMU 10.2.1 reference
and `tools/qemu_diff/`.

What did not survive verification:

- The browser end-to-end suite was described as 75 specs; there are **105**.
- Its milestone-ladder table was a *plan* — proposed checkpoints with proposed
  pass signatures. The ladder that exists in `scripts/win7.sh` has eight
  mechanical rungs keyed on architectural state, which is both real and better,
  so the built ladder is what `areas/testing.md` now documents. This also
  corrected a defect on that page, which had reproduced the aspirational ladder
  as though it were the implemented one.

What was absorbed:

- What Aero is, and the precedent survey behind "feasible but unprecedented" —
  v86, halfix, JSLinux, qemu-wasm, and the absence of any D3D→WebGPU precedent —
  plus the standing note that a 32-bit guest remains an unforeclosed fallback →
  `overview.md`, which had promised to say what Aero is and never did.
- The eight ladder rungs with the signal each depends on, why unreachable-state
  signatures cannot report a false pass, and the ~75-second QEMU+KVM
  ground-truth reference → `areas/testing.md`.
- The published overhead figures for comparable systems (WebAssembly ~1.45–1.55×
  native, TCG ~8×, CheerpX ~20× scalar, JSLinux ~50×) and the argument they
  support: the browser is not the floor, the execution tier is, so the JIT is the
  gate after the desktop milestone → `areas/performance.md`.
- That `--serial-out none` writes a file called `none` while `--debugcon-out`
  understands the keyword — still true in `aero-machine-cli` today →
  `areas/debugging.md`.
- Walk the whole path before deepening any part of it →
  `meta/engineering-principles.md`.

Deliberately not carried: every status claim in it. The document still described
a machine that died on `InvalidOpcode` after ~3,000 instructions, a boot blocked
at `0xc0000225`, an interpreter at 1.66 or 6.0 Minst/s, a `Starting Windows`
screen that had not moved in twenty-four minutes, and no `explorer.exe` desktop.
The desktop has since been reached and reproduced. Current state lives in
`../state/repo-state-and-structure.md` and current interpreter figures in
`../areas/performance.md`. Its "recommended next moves" and its list of packages
to install had all been executed or overtaken.

## The target-architecture design — ABSORB

`../decisions/0017-guest-driver-language.md`, `../decisions/0018-host-language.md`,
`../decisions/0001-repo-layout.md`, `../areas/build-and-tooling.md`,
`../state/repo-state-and-structure.md`

A root-level design document that laid out the structure the repo should
converge on, with cited research behind every toolchain choice. Unlike the other
root files this one held a great deal of durable material — but it held it in the
wrong shape: two standing architectural decisions were buried in a page marked
"draft for review", and `decisions/README.md` had to apologise for them living
outside the decisions directory. The version tables had also drifted from the
tree they described.

The two decisions are now first-class:

- [Guest driver language](../decisions/0017-guest-driver-language.md) — all
  Windows guest drivers stay C/C++, with the full
  `windows-drivers-rs` evidence and the three revisit conditions.
- [Host language](../decisions/0018-host-language.md) — a TypeScript shell over
  Rust workers, with the Leptos/Dioxus/Yew evaluation and the egui-on-wgpu option
  it leaves open.

The rest was verified against the tree before being carried, and a lot of it did
not survive:

| Claimed | Actually |
|---|---|
| wgpu at "~22-era" | **0.20.1** — the largest gap in the document |
| Node `>=22.11 <23`, npm workspaces | Node **24.18.0**, **pnpm 11.5.0**; no `package-lock.json` |
| 98 workspace members, mixed standalone lockfiles | **94** members, consolidation already done; one dead lockfile remains |
| Five named standalone Rust workspaces | All five joined the root workspace; only `fuzz/` is standalone, by cargo-fuzz's requirement |
| `resolver = "2"`, `[workspace.lints]` | Resolver is **"3"** (deliberately, MSRV-aware); `[workspace.lints]` does not exist |
| "eslint configs scattered" | There is **no JavaScript linter at all** — the cleanup task had no target |
| Go "retired" | **99 `.go` files remain**; the relay is unported and `services/webrtc-udp-relay/` does not exist |
| `tests/{e2e,contract}/` | `tests/e2e/` exists; contract tests are flat files in `tests/` |

Verification also turned up defects that were nobody's documentation problem:
`tools/win-offline-cert-injector/Cargo.lock` is dead because that crate is a root
workspace member, the ban list exists in three divergent copies that have to be
hand-synchronised, and `crates/legacy/README.md` contradicts the manifest about
workspace exclusion. These are recorded in `../areas/build-and-tooling.md` and
the state page's debt list rather than carried as prose.

One claimed defect did **not** survive a second check and is recorded here so it
is not rediscovered: the pnpm lockfile was reported as out of sync, listing six
importers against eleven workspace packages. It lists all twelve. The six that
looked absent have empty dependency sets, which is what their `package.json`
files actually declare. The lockfile is fine.

What was absorbed: the one-workspace-per-language rule and the corrected host
layout → `decisions/0001-repo-layout.md`; the verified pins, the deferred
majors with the cost of each lift, and the known toolchain defects →
`areas/build-and-tooling.md`; the remaining structural debt, including the
unported Go relay → `state/repo-state-and-structure.md`.

Not carried: the "current → target" absorption map and the execution order,
which were a work plan and are now either executed or recorded as debt; and the
target-tree diagram, which duplicated the state page's top-level map. That map
was corrected in the same pass — it still described the pre-merge `src/` and
`web/` trees as canonical.

## The cleanup plan — ABSORB

`../meta/working-agreements.md`, `./retirements.md`,
`../state/repo-state-and-structure.md`

The plan of record for the cleanup: the doctrine behind it, a classification of
every top-level directory and root file, the ban list, and a phase-by-phase
execution log. It had done its job — five of its six phases were executed and
gate-green — but it still described itself as *"plan of record, awaiting
execution go-ahead"* in its own header, and its body contradicted itself in
places (phase four reported the transport shims collapsed, then listed them as
open) and had visible editing damage in the decision log.

The durable parts were promoted out of it:

- The **scope doctrine** — what this repo is, that it is an agent-owned project
  rather than a simulated open-source foundation, and that noise is not neutral
  — and the **ban list** with its enforcement, and **absorb-not-bulldoze applied
  to code**, and the `.attic/` policy → `meta/working-agreements.md`, where the
  rest of the working doctrine already lived. A plan is the wrong home for a
  standing rule.
- The **deletion inventory** → the theater and legacy purge section above, so
  that the state page's history entries have something to point at.
- The **execution phases and decision log** were already recorded, dated, in the
  state page's history section; they were not carried twice.

Not carried: the classification legend (CANON / KEEP / CURATE / PURGE / BAN /
GENERATED), which was vocabulary for the cleanup exercise itself. The labels in
standing use are canonical, active, legacy and historical. Also not carried: the
per-directory classification tables, which were verdicts about a tree that has
since been reshaped by acting on them — several still listed `src/`, `web/` and
`emulator/` as current.

## The Windows 7 bring-up log — ABSORB

`./windows-7-bring-up.md`, `../areas/debugging.md`, `../areas/cpu-and-jit.md`,
`../areas/storage.md`, `../areas/windows-drivers.md`,
`../meta/engineering-principles.md`, `../meta/working-agreements.md`

Four thousand seven hundred lines of running, dated engineering log covering the
whole bring-up — roughly fifty sections and more than fifty numbered root causes.
It was by far the most valuable of the root-level files and the least usable: an
append-only record in which later entries routinely overturned earlier ones
without the earlier ones being edited, so a reader had to know the whole file to
trust any part of it. At least sixteen conclusions in it are explicitly
superseded by later conclusions in the same file, several of them load-bearing.

The curated record is now [windows-7-bring-up.md](./windows-7-bring-up.md),
organised by *class of defect* rather than by date, because the classes are what
generalise. Every regression test named in it was checked to still exist — all
thirty-one that were sampled do.

Where the rest went:

- The instrument reference — the windowed trace, the read and write watchpoints,
  the write stream, the physical and linear dumps, the framebuffer sampler, the
  firmware probe, snapshot resume-on-fault, and the technique for telling a slow
  run from a hung one — was **almost entirely missing from the wiki** and is now
  in `areas/debugging.md`, which is where the index had always promised it would
  be. So are the ways those instruments mislead, and the retired or hazardous
  overrides.
- The open JIT correctness defects → `areas/cpu-and-jit.md`. One of these is
  serious and unresolved: the compiled tier computes a wrong cryptographic digest
  in the guest, which stops a Windows service from starting, and the cause is not
  known. It was being managed by an oral rule in a scratch file.
- The device-protocol rules — the storage controller's programming interface and
  channel-enable register, the task-file shadow semantics, the two-interrupt
  packet protocol, and byte-granular scatter-gather — were mostly already in
  `areas/storage.md`; the scatter-gather rule was not, and is now.
- The inbox-driver survey, and the single-optical-slot constraint on unattended
  install → `areas/windows-drivers.md`.
- Establishing ground truth with a purpose-built oracle, and writing a regression
  that is observed red before it is made green → `meta/engineering-principles.md`.
- Evidence not living in scratch space, and separating what the guest did from
  what it was made to do → `meta/working-agreements.md`.

Two things were deliberately **not** carried. The dated narrative itself — the
hypothesis-by-hypothesis record of investigations whose conclusions were later
overturned — is not useful to a future reader and is actively misleading if read
in isolation; where a wrong turn taught something, the lesson was kept and the
narrative dropped. And the per-investigation resume recipes are gone, because
they are indexed by a lineage of hundreds of intermediate snapshots that only
mean anything inside the investigation that produced them.

To be precise about what did and did not survive, because an earlier draft of
this entry overstated it: the recipes citing the vanished `/tmp` scratch
directory are genuinely dead, but the lineages under the boot-output tree
are **not** — that tree still holds several hundred checkpoints, including the
QEMU ground-truth captures and every cold-boot milestone. It is now named on the
state page so a future session can find it.

One correction worth stating separately, because the log's own final entries
would mislead: its last stretch reaches a taskbar and a moving cursor through
extensive scaffolding, and is explicit that this was not a naturally booted
desktop. That work was **superseded** — the desktop was subsequently reached
without any of it. Both facts are in the curated record, in that order.

## The bring-up handoff — ABSORB

`../state/repo-state-and-structure.md`, `./windows-7-bring-up.md`,
`../meta/working-agreements.md`

The page that told a new session where to resume. It described itself as "a map,
not a duplicate", and had stopped being one: 2,877 lines, of which the largest
section was a dated ledger restating root causes already recorded in the bring-up
log, and the next largest a set of resume recipes citing a scratch directory that
no longer exists. Fifty-eight of its paths pointed into that directory. Its
closing sections — the working method, and an index of where knowledge lives —
duplicated `AGENTS.md` and `wiki/README.md` respectively.

Three things in it were genuinely load-bearing and are now in durable homes:

- **What the machine actually does today**, including that the desktop is reached
  and reproduced without scaffolding, the four environment settings that are
  configuration rather than wedges, and the fact that the browser path has a
  verified machine-to-scanout chain but no Windows guest → the state page.
- **The unscaffolded cold-boot baseline**, with its timing table and the
  half-hour-per-experiment figure that explains the whole snapshot-driven
  workflow → `history/windows-7-bring-up.md`.
- **The evidence problem** — that a large body of findings had become
  unverifiable when the scratch directory holding their proofs disappeared, and
  the rule that came out of it → `meta/working-agreements.md`, alongside the
  related rule about separating what the guest did from what it was made to do.

Also carried: the honest counting convention, which declined to mark the
input-path work closed while its host-side rules were fixed and tested but no
guest-visible change had been observed.

Not carried: the dated ledger (duplicated) and the method and index sections
(duplicated). The per-investigation resume recipes were dropped as an indexed
lineage rather than as dead paths — many of the checkpoints they name are still
on disk in the boot-output tree. What was kept is the recipe that
matters: how to resume the desktop, on the state page, with its disk image,
its paired overlay, and the two hazards that make a mismatch silently
worthless.

One exposure is recorded rather than resolved: the current desktop checkpoints
and their proof captures live in a working directory, not in the durable
snapshots location. Given that this exact failure has already cost this project
one body of evidence, moving them is outstanding work.

## Purge fallout — references that must be updated when `docs/` is deleted

Verified references to the retired sources from outside `docs/`/`wiki/`:

- `scripts/ci/check-repo-layout.sh:24` — `need_file "docs/repo-layout.md"`
  (hard requirement; will fail the layout check).
- `scripts/ci/check-repo-policy.sh:375-376` — "See also" comments citing
  `project-history.md` and `../areas/testing.md`.
- `deny.toml:12` — comment citing `project-history.md`.
- `tools/net-proxy-server/src/protocol.js:5` — comment citing
  `../specs/gateway-api.md`.
- README links to `../specs/gateway-api.md`:
  `services/gateway/README.md:79`, `services/net-proxy/README.md:269`,
  `protocol-vectors/README.md:57`, `tools/net-proxy-server/README.md:77`.
- `services/image-gateway/README.md:9,279` — links to
  `../areas/storage.md` (moot if `services/` is
  purged per the services-scope decision).

Repoint these to `wiki/history/retirements.md` (or the future areas pages
named above) in the same change that purges `docs/`.

## The second AeroGPU command executor — RETIRE

`crates/aero-gpu/src/acmd_executor.rs`

An 800-line Rust implementation of AeroGPU command-stream execution, sitting
alongside the TypeScript executor that the GPU worker actually uses.

Retired because it was not a competing implementation in any meaningful sense —
it was dead code. `AeroGpuAcmdExecutor` appeared nowhere outside its own file:
no consumer in any crate, no test, and no export through the WebAssembly bridge
that would let the browser reach it. The cleanup rule is to pick the superior
implementation and purge the residue; here only one of the two was ever
executed.

The live path is `apps/web/src/workers/aerogpu-acmd-executor.ts`, driven by
`apps/web/src/workers/gpu-worker.ts` and covered by its own test file.

## The compatibility alias crates — RETIRE

`crates/devices-compat`, `crates/platform-compat`, `crates/emulator-protocol-compat`

Three packages whose only purpose was to keep pre-rename package names
resolving, so that `cargo test -p devices` and friends kept working after the
canonical crates moved to `aero-` names.

Retired because they were costing a full second execution of the test suites
they aliased. Their test directories held 67 files: in `devices-compat` every
one was a two-line `#[path = "../../devices/tests/X.rs"] mod inner;` forwarder,
compiling and running the canonical crate's tests a second time under a
different package name. The handful that were not forwarders were equivalent to
the canonical crate's version modulo a comment or import formatting, and in one
case the canonical version was a strict superset.

No coverage was lost. `cargo test -p devices` now fails, which is correct: one
name per thing, per the crate-naming decision.
