# Networking

> How the Windows 7 guest reaches the network. A browser cannot open raw
> sockets, so a proxy is always in the data path; the decision that shapes
> everything else is that Aero tunnels raw Ethernet frames to it rather than
> running a TCP/IP stack in WebAssembly.
>
> The wire format is specified in
> [../specs/l2-tunnel-protocol.md](../specs/l2-tunnel-protocol.md), the gateway
> surface in [../specs/gateway-api.md](../specs/gateway-api.md), and the token
> formats in [../specs/auth-tokens.md](../specs/auth-tokens.md).

How the Windows 7 guest reaches the network. Browsers cannot open raw TCP/UDP
sockets, so a server-side proxy is always in the data path; the only design
question is where the host-side network stack lives.

**Decision:** tunnel raw Ethernet frames (L2) to an unprivileged proxy that
runs a user-space NAT stack ("slirp on the server"), instead of running a
TCP/IP stack in WASM. Recorded as the L2 tunnel networking decision
(`wiki/decisions/0013-networking-l2-tunnel.md`); the losing options (in-browser
slirp, L3/IP tunnel) are historical background only — see
[Legacy and retired](#legacy-and-retired).

## Current state (verified)

The L2 tunnel ("Option C") is the production path and is implemented:

- Browser client: `apps/web/src/net/l2Tunnel.ts` (`WebSocketL2TunnelClient`),
  exported from `apps/web/src/net/index.ts`. Optional WebRTC transport via
  `apps/web/src/net/l2RelaySignalingClient.ts` (`connectL2Relay`).
- Emulator-side backends: `crates/aero-net-backend` (`L2TunnelBackend` queue
  backend for native/test hosts; `L2TunnelRingBackend` bridging the
  `NET_TX`/`NET_RX` AIPC rings in the worker runtime). Re-exported for
  `crates/aero-net-backend`.
- NIC↔backend frame pump (bounded per-tick budgets): `crates/aero-net-pump`.
- Proxy: `crates/aero-l2-proxy` — WebSocket endpoint `GET /l2` (legacy alias
  `/eth`), user-space Ethernet/IP stack + NAT + egress policy. Listens on
  `AERO_L2_PROXY_LISTEN_ADDR` (default `0.0.0.0:8090`,
  `crates/aero-l2-proxy/src/config.rs`).
- Optional WebRTC transport: `proxy/webrtc-udp-relay` bridges a reliable,
  ordered `l2` DataChannel to the proxy's backend WebSocket
  (`L2_BACKEND_WS_URL`; see `proxy/webrtc-udp-relay/internal/webrtcpeer/l2_bridge.go`).
- Protocol crate (canonical constants/codec): `crates/aero-l2-protocol`
  (`crates/aero-l2-protocol/src/lib.rs`), mirrored in
  `apps/web/src/shared/l2TunnelProtocol.ts`,
  `services/gateway/src/protocol/l2Tunnel.ts`, and
  `proxy/webrtc-udp-relay/internal/l2tunnel/protocol.go`. These files carry
  "keep in sync" comments — treat the Rust crate as canonical.
- Cross-implementation test vectors:
  `crates/conformance/test-vectors/aero-vectors-v1.json` (`aero-l2-tunnel-v1` key).
- Smoke probe (ARP + DNS + TCP echo over the real framing):
  `tests/networking-architecture-rfc.test.js` with the protocol helper
  `tests/helpers/l2_tunnel_proto.js`.

In the worker-based web runtime the net worker (`apps/web/src/workers/net.worker.ts`)
is L2-tunnel-only: it forwards raw frames between the `NET_TX`/`NET_RX` rings
(`apps/web/src/runtime/shared_layout.ts`, `IO_IPC_NET_*`, 512 KiB ring capacity) and
the tunnel transport. The tunnel is selected by configuring `proxyUrl`
(default unset) in `apps/web/src/config/aero_config.ts`, with
`l2TunnelTransport: "ws" | "webrtc"` and optional `l2TunnelToken` /
`l2TunnelTokenTransport` knobs. The guest-facing NICs are E1000
(`crates/aero-net-e1000`) and virtio-net (preferred once guest drivers are
installed; mode selectable via `virtioNetMode` in `apps/web/src/config/aero_config.ts`;
contract: `AERO-W7-VIRTIO` in `wiki/areas/windows-drivers.md`).

## Data path

```
Win7 guest (tcpip.sys → NDIS → e1000/virtio-net driver)
  → NIC device model (crates/aero-net-e1000, virtio-net)
  → aero-net-backend ring backend (NET_TX/NET_RX AIPC rings)
  → net worker (apps/web/src/workers/net.worker.ts) — pure frame forwarder
  → WebSocket /l2 (default) or WebRTC DataChannel "l2" (optional)
  → aero-l2-proxy: DHCP/DNS on a synthetic LAN + NAT + egress policy
  → host TCP/UDP sockets → Internet
```

The browser never parses ARP/DHCP/IP/TCP/UDP on this path; it is an L2 pipe.

## Wire contract: `aero-l2-tunnel-v1`

Transport-independent, message-oriented framing (same bytes over WebSocket and
WebRTC). Constants below are verified against `crates/aero-l2-protocol/src/lib.rs`.

- Negotiation (WebSocket): subprotocol `aero-l2-tunnel-v1` is mandatory; the
  connection is rejected if not negotiated. (WebRTC): DataChannel label `l2`.
- WebRTC reliability: the `l2` channel MUST be fully reliable and ordered
  (`ordered = true`, `maxRetransmits`/`maxPacketLifeTime` unset). Rationale:
  the proxy ACKs upstream TCP before the guest receives it, and the proxy-side
  TCP termination assumes in-order guest segments. The relay closes unordered
  `l2` channels (`proxy/webrtc-udp-relay/PROTOCOL.md`).
- Header: 4 bytes — magic `0xA2`, version `0x03` (`0x02` is reserved for the
  UDP relay v2 prefix), type `u8`, flags `u8` (senders set 0; receivers ignore
  unknown bits).
- Message types: `0x00 FRAME` (one raw Ethernet frame, opaque payload),
  `0x01 PING` / `0x02 PONG` (keepalive/RTT; PONG echoes the PING payload;
  recommended payload is a u64 BE millisecond timestamp), `0x7F ERROR`.
- ERROR payload: either a UTF-8 string or structured binary
  `code(u16 BE) | msg_len(u16 BE) | msg`. Stable codes 1–9:
  `protocol_error`, `auth_required`, `auth_invalid`, `origin_missing`,
  `origin_denied`, `quota_bytes`, `quota_fps`, `quota_connections`,
  `backpressure` (`L2_TUNNEL_ERROR_CODE_*`). The Rust proxy sends the
  structured form then closes with WebSocket code 1008.
- Size limits (configurable; defaults): FRAME payload ≤ 2048 bytes, control
  payloads ≤ 256 bytes (`L2_TUNNEL_DEFAULT_MAX_FRAME_PAYLOAD`,
  `L2_TUNNEL_DEFAULT_MAX_CONTROL_PAYLOAD`). The proxy derives its WebSocket
  `max_message_size` from these so oversized messages are rejected before
  buffering.
- Robustness: drop messages shorter than the header, with bad magic/version,
  or over the size cap; ignore unknown types; close the connection after a
  configurable threshold of repeated violations.

### Security model (normative)

The tunnel is an Internet egress path and must be deployed like `/tcp`:
Origin allowlist + authentication on the upgrade, egress policy in the proxy
(deny private/reserved ranges by default, port allowlists, per-session
quotas), and never bridge the tunnel to the host LAN — frames terminate in the
proxy's synthetic L2 segment.

## Auth tokens (cross-language contract)

Minted/verified by Rust (`crates/aero-auth-tokens`, canonical), TypeScript
(`services/gateway/src/session.ts`, `.../udpRelay.ts`), and Go
(`proxy/webrtc-udp-relay/internal/auth/jwt.go`). Negative-case vectors:
`protocol-vectors/auth-tokens.json`.

- **Gateway session token** (`aero_session` cookie):
  `<payload_b64url_no_pad>.<sig_b64url_no_pad>`,
  `sig = HMAC_SHA256(secret, payload_segment)`. Payload requires `v: 1`,
  non-empty `sid`, `exp` (seconds since epoch; expired when
  `nowMs ≥ exp×1000`).
- **UDP relay HS256 JWT**: standard 3-segment compact form, header must have
  `alg: "HS256"`; payload requires `sid`, `iat`, `exp`, optional
  `origin`/`aud`/`iss`/`nbf`.
- Strict verification contract (all tokens): base64url **without padding**;
  reject `len % 4 == 1` and non-canonical encodings (non-zero unused bits);
  exact segment counts; no empty segments; size caps enforced **before**
  decoding; signature segment exactly 43 chars (32-byte HMAC-SHA256); verify
  the HMAC **before** parsing JSON claims.

## Origin allowlist (shared contract)

The L2 proxy, gateway, and WebRTC relay share origin-normalization semantics:
compare `<lowercase-scheme>://<lowercase-host>[:port]`, normalize away default
ports (`:80` http, `:443` https), accept only `http`/`https` origins with no
credentials, query, fragment, or non-`/` path; `null` only when explicitly
configured (or under `*`).

The canonical normalization vectors live in
`protocol-vectors/origin.json` and are **consumed directly by
tests** — `services/gateway/test/originGuard.test.ts` and
`proxy/webrtc-udp-relay/internal/origin/origin_test.go` load it by relative
path. It is data, not prose: do not move it without updating both tests.

## Configuration

### L2 proxy (`crates/aero-l2-proxy`, env-driven; see `src/config.rs`)

- Listen: `AERO_L2_PROXY_LISTEN_ADDR` (default `0.0.0.0:8090`).
- Origin: `AERO_L2_ALLOWED_ORIGINS` (falls back to shared `ALLOWED_ORIGINS`;
  `AERO_L2_ALLOWED_ORIGINS_EXTRA` appended; `*` allows any *valid* Origin).
  `AERO_L2_OPEN=1` disables Origin enforcement (trusted local dev only).
- Auth: `AERO_L2_AUTH_MODE` = `none | session | token | jwt | cookie_or_jwt |
  session_or_token | session_and_token` (legacy aliases `cookie`, `api_key`,
  `cookie_or_api_key`, `cookie_and_api_key`). Secrets:
  `AERO_GATEWAY_SESSION_SECRET` (preferred) / `SESSION_SECRET` /
  `AERO_L2_SESSION_SECRET` (legacy) for session-cookie verification;
  `AERO_L2_API_KEY` (legacy alias `AERO_L2_TOKEN`) for token mode;
  `AERO_L2_JWT_SECRET` (+ optional `AERO_L2_JWT_AUDIENCE` /
  `AERO_L2_JWT_ISSUER`) for JWT. Safe default when unset: `session_or_token`
  if a session secret exists, else `token` if a key exists, else refuse to
  start unless `AERO_L2_OPEN=1` and `AERO_L2_INSECURE_ALLOW_NO_AUTH=1`.
- Credential delivery: `aero_session` cookie; `?token=` / `?apiKey=` query;
  `Authorization: Bearer` (JWT); or an extra `Sec-WebSocket-Protocol` entry
  `aero-l2-token.<credential>` offered alongside `aero-l2-tunnel-v1`
  (preferred — keeps secrets out of URLs/logs; credential must be a valid
  RFC 7230 token). Bad credentials → HTTP 401, no upgrade.
- Quotas: `AERO_L2_MAX_CONNECTIONS` (default 64, 0 disables; excess → 429),
  `AERO_L2_MAX_CONNECTIONS_PER_SESSION` (legacy alias
  `AERO_L2_MAX_TUNNELS_PER_SESSION`), `AERO_L2_MAX_BYTES_PER_CONNECTION`,
  `AERO_L2_MAX_FRAMES_PER_SECOND` (0 = unlimited; excess → WS close 1008).
- Payload caps: `AERO_L2_MAX_FRAME_PAYLOAD` (default 2048; legacy alias
  `AERO_L2_MAX_FRAME_SIZE`), `AERO_L2_MAX_CONTROL_PAYLOAD` (default 256).
  `services/gateway` surfaces these via `POST /session`, so set them on
  the gateway too when overriding.
- Observability: `AERO_L2_CAPTURE_DIR` (per-session `.pcapng`),
  `AERO_L2_CAPTURE_MAX_BYTES` (default 64 MiB), `AERO_L2_CAPTURE_FLUSH_INTERVAL_MS`
  (default 1000), `AERO_L2_PING_INTERVAL_MS` (proxy-initiated PINGs; RTT in
  metrics).

### WebRTC relay (`proxy/webrtc-udp-relay`; see `internal/config/config.go`)

- L2 bridge: `L2_BACKEND_WS_URL` (e.g. `ws://aero-l2-proxy:8090/l2`),
  `L2_BACKEND_AUTH_FORWARD_MODE=query|subprotocol|none`, `L2_BACKEND_TOKEN`,
  `L2_BACKEND_FORWARD_ORIGIN`, `L2_BACKEND_FORWARD_AERO_SESSION` (forward the
  session cookie captured at signaling; needed when the proxy uses
  `AERO_L2_AUTH_MODE=session`), `L2_BACKEND_ORIGIN_OVERRIDE`.
- Hardening: `UDP_INBOUND_FILTER_MODE=address_and_port` (default; `any` is
  full-cone, less safe), `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` (SDP hint),
  `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES` (hard receive cap),
  `WEBRTC_SESSION_CONNECT_TIMEOUT` (default 30s; closes half-open sessions).
- `GET /webrtc/ice` responses are non-cacheable (`Cache-Control: no-store`).

### Gateway TCP egress policy (`services/gateway`)

`TCP_ALLOWED_HOSTS` / `TCP_BLOCKED_HOSTS` (exact + `*.example.com` wildcard
patterns; deny overrides allow), `TCP_REQUIRE_DNS_NAME=1` to reject IP
literals. Decisions apply pre-DNS; private/reserved blocking re-applies
post-DNS (first allowed public IP wins).

### Local dev

```bash
cargo run --locked -p aero-l2-proxy          # add AERO_L2_OPEN=1 for trusted local dev
AERO_PROXY_OPEN=1 pnpm -C services/net-proxy run dev   # Phase-0 dev relay (DoH etc.), port 8081
node --test tests/networking-architecture-rfc.test.js   # L2 probe (ARP+DNS+TCP echo)
```

Browser side: `new WebSocketL2TunnelClient("http://127.0.0.1:8090", sink)` —
http(s) URLs are auto-converted to ws(s) with `/l2` appended; pass
`{ token, tokenTransport: "subprotocol" }` when the proxy requires auth.

## Snapshot / restore

- Snapshot entries: `DeviceId::E1000` (19, kind `net.e1000`) and
  `DeviceId::NET_STACK` (20, kind `net.stack`) — `crates/aero-snapshot/src/format.rs`,
  kind strings in `crates/aero-wasm/src/vm_snapshot_device_kind.rs`.
- The NIC device model state is bit-restored; host transports (tunnel
  connections, shared rings) are external and are **not** restorable. Expect
  occasional packet loss right after restore; guest TCP retransmits cope.
- In-browser stack (`net.stack`, Phase 0): restore policy is **Drop** —
  learned guest MAC / IP-assigned flag / next-id counters / bounded DNS cache
  are kept; in-flight DNS and all TCP connections are dropped.
  `TcpRestorePolicy::Reconnect` exists (`crates/aero-net-stack`) as an
  explicit best-effort mode but cannot preserve TCP semantics (breaks TLS);
  canonical snapshots use Drop.
- L2 tunnel: on resume the forwarder reconnects best-effort (backoff with
  jitter, `apps/web/src/workers/net.worker.ts`). Ordering: pause CPU+I/O, then
  pause NET, close the transport, drain/reset `NET_TX`/`NET_RX` **after** all
  producers/consumers are paused, snapshot, then resume. The net worker
  participates via `vm.snapshot.pause` / `vm.snapshot.resume`.

## Observability

- Forwarder telemetry: the net worker emits ~1Hz `[net] l2: ...` log events
  (tx/rx frame+byte counters, drop deltas `rx_full`/`pending`/`tx_bp`) while
  `proxyUrl` is configured; transitions (`connecting/open/closed/error`) log
  immediately; non-zero drop deltas escalate to WARN; transport errors to
  ERROR. See `apps/web/src/net/l2TunnelForwarder.ts`, `apps/web/src/net/l2TunnelTelemetry.ts`.
- PCAPNG tracing (browser): capture at the L2 forwarder boundary, one
  `guest-eth0` interface with direction in `epb_flags`; optional `tcp-proxy` /
  `udp-proxy` pseudo-interfaces (`LINKTYPE_USER0/1`). UI panel "Network trace
  (PCAPNG)" plus `window.aero.netTrace` (`shared/aero_api.ts`,
  `apps/web/src/net/trace_backend.ts`, `apps/web/src/net/net_tracer.ts`,
  `apps/web/src/net/pcapng.ts`). 16 MiB in-memory cap with drop counters;
  `downloadPcapng()` drains, `exportPcapng()` snapshots, `clear()` resets
  counters. Captures may contain credentials — treat like secrets; tracing
  defaults off. Rust-side equivalent: `crates/aero-net-trace` (`NetTracer`,
  optional `NetTraceRedactor`).
- Proxy side: `GET /healthz`, `/readyz`, `/version`, `/metrics` (Prometheus)
  on both `aero-l2-proxy` (`crates/aero-l2-proxy/src/server.rs`) and the
  gateway. L2 capture metrics: `l2_capture_frames_total`,
  `l2_capture_bytes_total`, `l2_capture_frames_dropped_total`,
  `l2_capture_errors_total`; RTT histogram `l2_ping_rtt_ms` when
  `AERO_L2_PING_INTERVAL_MS` is set. Expose these only on trusted networks.

### Where an undeliverable datagram gets counted

`l2_udp_send_fail_total` counts datagrams the proxy could not deliver, and
getting that count right takes a little care, because the kernel does not report
the failure where you would expect it.

A UDP flow is a connected socket plus a task parked in `recv`. When the far end
answers with ICMP unreachable, the kernel queues the error against the socket and
hands it to whichever operation runs next. The receive task is already parked
waiting, so it wins that race essentially every time — and it wins it *before*
the guest's next datagram is ever sent.

Reacting to the error there is tempting and wrong in two ways. Closing the flow
means the guest's next datagram leaves from a fresh source port, which breaks any
peer that keyed on the old one; a name server or game server that restarts
produces exactly this and then resumes. And swallowing the error silently means
the send path — the only place an undeliverable datagram can honestly be
attributed — never learns about it, so the counter stays at zero no matter how
many datagrams fall on the floor.

The receive task therefore records the condition on the flow and keeps going.
The next send sees the mark, counts the failure, and tears the flow down
deliberately. A successful receive clears the mark, so a peer that comes back
keeps its flow. Errors that are not ICMP-derived still close the flow
immediately, and a bounded run of consecutive transient errors closes it too, so
a platform that latches the error cannot turn the receive loop into a spin.

## Legacy and retired

- **Phase 0 in-browser slirp/NAT** (`crates/aero-net-stack`): ARP/DHCP + TCP/UDP NAT in
  WASM, TCP egress over gateway `WS /tcp` or `/tcp-mux` (subprotocol
  `aero-tcp-mux-v1`, 9-byte big-endian header; impls
  `services/gateway/src/protocol/tcpMux.ts`,
  `services/net-proxy/src/tcpMuxProtocol.ts`, `apps/web/src/net/tcpMuxProxy.ts`), DNS over
  DoH `/dns-query` (+ optional `/dns-json`), UDP over WebRTC DataChannel
  `udp` (`ordered=false`, `maxRetransmits=0`) or `WS /udp` fallback. Kept as
  the development/debug fallback; not the production direction.
- **UDP relay framing v1/v2** (`proxy/webrtc-udp-relay/PROTOCOL.md`; Go impl
  `proxy/webrtc-udp-relay/internal/udpproto/`, browser
  `apps/web/src/shared/udpRelayProtocol.ts`): v2 frames start `0xA2 0x02`; anything
  else is legacy IPv4-only v1. IPv6 requires v2. The `/udp` WebSocket fallback
  is reliable+ordered, so it cannot reproduce UDP loss/reordering — debug
  path only.
- **`services/net-proxy/`**: local dev relay for the Phase 0 endpoints
  (`AERO_PROXY_OPEN`, `AERO_PROXY_ALLOW`, `AERO_PROXY_DOH_CORS_ALLOW_ORIGINS`;
  legacy per-target `WS /udp?host=...&port=...` mode exists — prefer
  multiplexed v1/v2 framing). `tools/net-proxy-server/` speaks the same mux
  framing with `?token=` auth.
- **`server/`**: the legacy monolith backend referenced by older docs is gone
  from the tree (verified); backend work lives in `services/gateway`,
  `proxy/webrtc-udp-relay`, `crates/aero-l2-proxy`, `crates/aero-storage-server`.
- **Retired:** `crates/aero-net` (Tokio-era experiment; only a README stub
  remains, not in the workspace). The emulator-native stack wrappers
  are also gone — the whole second device stack that carried them has been
  retired
  and tracing.
  The docs' references to `tools/aero-gateway-rs` (legacy Rust gateway
  prototype) and `deploy/docker-compose.yml` are stale — both are gone; the
  relay compose stack lives at `proxy/webrtc-udp-relay/docker-compose.yml`.
- **RFC** (absorbed from `networking.md`): superseded
  by the L2 tunnel networking decision; the option-A/B/C tradeoff analysis is preserved in
  `wiki/decisions/0013-networking-l2-tunnel.md`.

## Open issues / debt

- The L2 tunnel is opt-in via `proxyUrl` config; "default in production
  builds, slirp behind a debug flag" (RFC migration phases 2–3) is
  [aspirational] — there is no build-time switch in the repo.
- Continued hardening of proxy egress policy, observability, and resource
  accounting under real workloads (per the RFC) — ongoing.
- The automated probe (`tests/networking-architecture-rfc.test.js`) covers
  ARP + DNS + TCP echo but not DHCP; DHCP is verified manually in a booted
  guest (`ipconfig /all`, `/renew`) with PCAPNG tracing.
- `protocol-vectors/origin.json` is the vector file's home — test data
  loaded by gateway and relay tests, relocated out of `docs/` ahead of the
  purge (see [Origin allowlist](#origin-allowlist-shared-contract)).

## Pointers

- the L2 tunnel networking decision: `wiki/decisions/0013-networking-l2-tunnel.md`
- L2 wire protocol: this page (constants canonical in
  `crates/aero-l2-protocol`)
- Ops/runbook detail: this page (absorbed from `networking.md`)
- Gateway HTTP/WS API: `wiki/history/retirements.md` (absorbed from the
  retired `docs/backend/` API docs; gateway code at `services/gateway`)
- UDP relay + signaling + `l2` bridge: `proxy/webrtc-udp-relay/PROTOCOL.md`
- IPC rings (`NET_TX`/`NET_RX`): `wiki/areas/platform-and-firmware.md`,
  `apps/web/src/runtime/shared_layout.ts`
- Snapshot device IDs: `wiki/areas/platform-and-firmware.md`,
  `crates/aero-snapshot/src/format.rs`
- Virtio contract: `wiki/areas/windows-drivers.md` (`AERO-W7-VIRTIO`)

## Device-model fixes applied (2026-07-26/27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §6 for full details.

- **E1000 TSO context descriptor**: MSS and HDR_LEN byte offsets were swapped
  (MSS at bytes 12-13, HDR_LEN at 14; correct: MSS at 14-15, HDR_LEN at 13).
  Every LSO TCP segment was assembled with wrong MSS → oversized frames silently
  dropped → all bulk TCP stalled.
- **E1000 PHY BMSR**: missing ANEGCAPABLE + speed capability bits → driver
  couldn't complete PHY auto-negotiation setup.
- **E1000 ICR**: read now clears only unmasked bits (per spec); added LSC, TXQE,
  RXDMT0, RXO interrupt cause bits; LSC raised on reset for initial link-up.
- **E1000 EECD**: AUTO_RD bit set on reset (signals EEPROM auto-loaded).
- **E1000 TCTL.PSP**: short TX frames (< 60 bytes) now padded to Ethernet minimum.
- **TCP MSS segmentation** (`aero-net-stack`): proxy→guest data now chunked into
  1460-byte segments (was one giant segment up to 16 KiB → dropped by L2 encoder
  2048-byte limit + E1000 1522-byte limit, with send sequence advanced past the
  undelivered data → permanent connection stall).
- **Remaining**: no TCP retransmission buffer (any dropped proxy→guest frame
  permanently desyncs the connection); guest FIN treated as full teardown (no
  half-close); WS backpressure kills the session (should throttle instead).

## Networking Stack

### Overview

Aero needs to expose a “real” NIC to the Windows 7 guest while running inside a browser that cannot
open arbitrary TCP/UDP sockets. That means *some* proxy service is always in the data path; the
primary question is where the “host-side” networking stack lives.

### Current recommended architecture (Option C: L2 tunnel to proxy)

**Final decision:** [the L2 tunnel networking decision: Networking via L2 tunnel (Option C) to an unprivileged proxy](../decisions/0013-networking-l2-tunnel.md).

**Option C (L2 tunnel) is the recommended production architecture.** It keeps browser CPU usage low
and avoids implementing a TCP/IP stack in WASM.

**Summary:**

- **Browser:** pure Ethernet frame forwarder (an L2 pipe).
  - The browser does not parse ARP/DHCP/IP/TCP/UDP; it forwards raw frames.
- **Proxy:** unprivileged user-space NAT stack (“slirp on the server”).
  - The proxy terminates Ethernet, provides DHCP/DNS on a synthetic LAN, and opens host sockets for
    outbound TCP/UDP.
- **Transport:** **WebSocket first** (single reliable tunnel), **WebRTC optional** (DataChannel-based
  tunnel). For WebRTC, the `l2` DataChannel MUST be reliable and ordered (`ordered = true`).

#### End-to-end data path

```
Windows 7 guest
  TCP/IP stack + DHCP client
        │
        ▼
Virtual NIC (e1000 / virtio-net)
        │   (Ethernet frames)
        ▼
WASM emulator (Rust)
  L2 tunnel backend (frame pipe)
    - `L2TunnelRingBackend` (browser runtime): `NET_TX`/`NET_RX` AIPC rings
        │   (Ethernet frames)
        ▼
Browser transport
  WebSocket (default) / WebRTC DataChannel (optional)
        │
        ▼
Proxy: aero-l2-proxy
  user-space Ethernet+IP stack + NAT + policy
        │   (host TCP/UDP sockets)
        ▼
Internet
```

Browser runtime note: in the worker-based web runtime, the emulator exchanges raw Ethernet frames
with the JS tunnel client via `SharedArrayBuffer` AIPC rings (`NET_TX`/`NET_RX`) inside `ioIpcSab`.
See `../specs/worker-ipc-protocol.md` (queue kinds) and `apps/web/src/runtime/shared_layout.ts` (`IO_IPC_NET_*`).

#### Browser-side observability (L2 forwarder telemetry)

The network worker periodically emits low-rate runtime `log` events summarizing the L2 tunnel
forwarder state and drop counters. These surface in the dev console with the coordinator's role
prefix, e.g.:

```
[net] l2: open tx=10f/100B rx=20f/200B drop+{rx_full=0, pending=1, tx_bp=0} pending=0f/0B
```

- Logs are emitted at ~1Hz when `proxyUrl` is configured.
- Transition logs (`l2: connecting/open/closed/error`) are emitted immediately.
- When drop deltas are non-zero, the periodic stats log is emitted at `WARN` so it also appears in
  the coordinator's nonfatal event stream.
- Tunnel transport errors are emitted at `ERROR` (`l2: error: ...`).

For packet-level debugging (Wireshark), see **Network Tracing (PCAP/PCAPNG Export)** below.

See:
- `apps/web/src/net/l2TunnelForwarder.ts` (counters + log formatting helper)
- `apps/web/src/workers/net.worker.ts` (periodic + transition log emission)

#### Key docs and repo components

- Background/tradeoffs: [`networking-architecture-rfc.md`](networking.md)
- Wire protocol: [`l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md)
- Runbook (local + production): [`l2-tunnel-runbook.md`](networking.md)

Authoritative endpoint/protocol docs (browser-facing contracts):

- **Aero Gateway HTTP API:** [`services/gateway/openapi.yaml`](../../services/gateway/openapi.yaml)
  - `/session`, `/dns-query`, `/dns-json`, `/udp-relay/token`, etc.
- **Aero Gateway WebSocket API:** [`../specs/gateway-api.md`](../specs/gateway-api.md)
  - `/tcp` and `/tcp-mux` (`aero-tcp-mux-v1`)
- **WebRTC UDP relay (+ `/udp` WebSocket fallback + `l2` bridge):**
  [`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md)

Planned/active code paths:

- Browser tunnel client: `apps/web/src/net/l2Tunnel.ts`
- L2 tunnel backends (queue-backed + ring-backed NET_TX/NET_RX bridge): `crates/aero-net-backend`
- NIC↔backend frame pump glue (bounded per-tick budgets, deterministic ordering): `crates/aero-net-pump`
- WebSocket L2 proxy (unprivileged): `crates/aero-l2-proxy`
- WebRTC transport (optional): `proxy/webrtc-udp-relay` (DataChannel carrying the L2 tunnel)

### Migration from in-browser slirp/NAT

The migration is structured so we can ship Option C incrementally without regressing networking for
contributors.

#### Phase 0 (current): in-browser slirp/NAT using `/tcp` + UDP relay

- Browser runs a slirp-like stack (ARP/DHCP + TCP/UDP NAT).
- TCP egress is implemented via the gateway’s WebSocket endpoints (`/tcp` or `/tcp-mux`,
  subprotocol `aero-tcp-mux-v1`).
- DNS resolution uses DoH endpoints (`/dns-query`, optional `/dns-json`).
   - Production deployments expose these via `services/gateway`.
   - Local dev can use `net-proxy` which implements the same DoH paths; see [`services/net-proxy/README.md`](../decisions/README.md)
     (including notes on same-origin fetch/CORS and the optional `AERO_PROXY_DOH_CORS_ALLOW_ORIGINS` allowlist).
- UDP egress is implemented via `proxy/webrtc-udp-relay` (WebRTC DataChannel `udp`, with `/udp`
  WebSocket fallback; see `proxy/webrtc-udp-relay/PROTOCOL.md`).
  - Inbound UDP filtering: by default, the relay only forwards inbound UDP from remote address+port
    tuples that the guest previously sent to (`UDP_INBOUND_FILTER_MODE=address_and_port`). This is
    safer for public deployments. You can switch to full-cone behavior with
    `UDP_INBOUND_FILTER_MODE=any` (**less safe**; accepts inbound UDP from any remote endpoint).
  - DoS hardening: the relay configures pion/SCTP message-size caps to prevent malicious peers from
    sending extremely large WebRTC DataChannel messages that would otherwise be buffered/allocated
    before `DataChannel.OnMessage` runs. See:
    `WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` (SDP hint) and `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES`
    (hard receive-side cap).
  - Session leak hardening: the relay closes server-side PeerConnections that never connect within
    `WEBRTC_SESSION_CONNECT_TIMEOUT` (default `30s`).

#### Phase 1: introduce `L2TunnelBackend` (frame pipe) and keep slirp as fallback

- Add an `L2TunnelBackend` that forwards raw Ethernet frames to a proxy over WebSocket.
- Keep the in-browser slirp/NAT stack as a fallback for development and debugging.
- Ensure snapshots/restore continue to work by capturing tunnel connection bookkeeping and treating
  active connections as non-restorable (same policy as today).

#### Phase 2: default to the L2 tunnel in production builds

- Production builds select the L2 tunnel path by default.
- Legacy slirp remains available behind a debug flag for bisecting and emergency fallback.

#### Phase 3: retire in-browser TCP NAT (keep only as debug fallback)

- Remove the CPU-heavy in-browser TCP NAT path from the normal build.
- Keep a minimal debug-only fallback for isolating issues (ideally off by default and not shipped
  for public deployments).

For performance, **virtio-net** is the preferred paravirtualized NIC once virtio drivers are installed. Under Aero’s Windows 7 virtio contract ([`AERO-W7-VIRTIO` v1](../specs/windows7-virtio-driver-contract.md)), virtio devices are exposed as **virtio-pci modern-only** (virtio 1.0+) via PCI vendor-specific capabilities and a single **BAR0 MMIO** register region.

Compatibility note: virtio-pci legacy/transitional (I/O port BAR) support may be desirable for some **upstream virtio-win** driver bundles, but it is **not required** by the Aero contract and is treated as an optional mode. In the web runtime, select this via the Settings UI ("Virtio-net mode") or set `virtioNetMode` in config/URL query (`?virtioNetMode=transitional|legacy`). Similar optional transport selectors exist for virtio-input and virtio-snd; see: [`16-virtio-pci-legacy-transitional.md`](windows-drivers.md)

---

> Note: The sections below primarily describe the Phase 0 in-browser slirp/NAT stack (legacy /
> fallback). The production path is the L2 tunnel described above.

### Network Architecture

```
┌───────────────────────────────────────────────────────────────────────────────┐
│                                Windows 7 guest                                 │
│                                                                               │
│  apps → Winsock → tcpip.sys → NDIS → NIC driver (e1000e.sys / virtio-net.sys)  │
└───────────────────────────────────────────────────────────────────────────────┘
                 │  ◄── emulation boundary (guest ↔ browser/WASM)
                 ▼
┌───────────────────────────────────────────────────────────────────────────────┐
│                            Browser (Aero emulator)                             │
│                                                                               │
│  Phase 0 (fallback): in-browser slirp/NAT (`crates/aero-net-stack`)            │
│    - TCP  → WebSocket `/tcp` (or `/tcp-mux` + `aero-tcp-mux-v1`)               │
│    - DNS  → DoH `/dns-query` (optional `/dns-json`)                            │
│    - UDP  → WebRTC DataChannel `udp` (or WebSocket `/udp` fallback)            │
└───────────────────────────────────────────────────────────────────────────────┘
                 │ HTTPS / WSS (same-site recommended; cookies)     │ WebRTC (UDP)
                 │                                                    │
                 ▼                                                    ▼
┌──────────────────────────────────────────────┐     ┌──────────────────────────────────────────────┐
│ services/gateway                          │     │ proxy/webrtc-udp-relay                        │
│  - POST /session                              │     │  - WebRTC signaling (see PROTOCOL.md)         │
│  - WS  /tcp (1 TCP stream per WS)             │     │    - GET  /webrtc/signal (WS, trickle ICE)     │
│  - WS  /tcp-mux (subprotocol aero-tcp-mux-v1) │     │    - POST /webrtc/offer (HTTP offer→answer)    │
│  - DoH /dns-query (+ optional /dns-json)      │     │  - UDP relay: DataChannel `udp` + WS /udp      │
│  - POST /udp-relay/token (optional)           │     └──────────────────────────────────────────────┘
└──────────────────────────────────────────────┘
```

Production deployment commonly puts a reverse proxy (e.g. Caddy) in front so the browser only
talks to a single HTTPS origin:

- `/session`, `/tcp`, `/tcp-mux`, `/dns-query`, `/dns-json`, `/udp-relay/token` → **aero-gateway**
- `/webrtc/*`, `/offer`, `/udp` → **webrtc-udp-relay**
  - `GET /webrtc/ice` responses may include TURN credentials and are explicitly **non-cacheable** (`Cache-Control: no-store`, `Pragma: no-cache`, `Expires: 0`) to avoid proxy/browser caching leaks and stale credentials.
- `/l2` (legacy alias: `/eth`) → **aero-l2-proxy** (Option C; `aero-l2-tunnel-v1`)

Auth note (Option C): `/l2` should be treated like `/tcp` — for any internet-exposed deployment you
should enable **Origin allowlisting + authentication**. The Rust L2 proxy (`crates/aero-l2-proxy`)
enforces an Origin allowlist by default and
supports multiple auth modes via `AERO_L2_AUTH_MODE`:

- `session` (recommended for same-origin browser clients): requires the `aero_session` cookie issued
  by `POST /session`. The proxy must share the gateway session signing secret (`SESSION_SECRET`) via
  `AERO_GATEWAY_SESSION_SECRET` (preferred), or `SESSION_SECRET`, or `AERO_L2_SESSION_SECRET` (legacy), so it can verify the cookie.
- `token` / `jwt` / `cookie_or_jwt` / `session_or_token` / `session_and_token`: useful for cross-origin deployments and non-browser/internal
  clients. Credentials can be delivered via:
  - query param: `?token=...` (or `?apiKey=...` for compatibility),
  - `Authorization: Bearer <token>` (JWT), or
  - an additional `Sec-WebSocket-Protocol` entry `aero-l2-token.<credential>` (offered alongside
    `aero-l2-tunnel-v1`; requires the credential be valid for the WebSocket subprotocol token grammar;
    prefer this form when possible to avoid putting secrets in URLs/logs).
  - Optional JWT validation: set `AERO_L2_JWT_AUDIENCE` and/or `AERO_L2_JWT_ISSUER` (when set, claims must match).
- Compatibility aliases: `cookie`→`session`, `api_key`→`token`, `cookie_or_api_key`→`session_or_token`,
  `cookie_and_api_key`→`session_and_token`.
- `AERO_L2_TOKEN` is a legacy alias for token auth (used when `AERO_L2_AUTH_MODE` is unset and also
  accepted as a fallback value for `AERO_L2_API_KEY` in `token` mode; legacy alias `api_key`).

When using the gateway session bootstrap, prefer `endpoints.l2` from `POST /session` instead of
hardcoding `/l2`.

Note: WebRTC’s **data plane** still requires UDP connectivity to the relay’s ICE port range (or a
TURN server). The reverse proxy only fronts the relay’s HTTP/WebSocket signaling endpoints.

---

### Canonical protocols (wire formats)

This section summarizes the canonical on-the-wire formats. The full specifications live in the
linked documents.

#### `aero-tcp-mux-v1` (TCP multiplexing over WebSocket)

Used by `GET /tcp-mux` (WebSocket) on `services/gateway` (and the dev relays).

- **Spec:** [`../specs/gateway-api.md`](../specs/gateway-api.md)
- **Reference implementations:**
  - `services/gateway/src/protocol/tcpMux.ts`
  - `services/net-proxy/src/tcpMuxProtocol.ts`
  - `apps/web/src/net/tcpMuxProxy.ts`

Frame format (big-endian), fixed **9-byte header**:

| Field | Type | Notes |
|---|---:|---|
| `msg_type` | `u8` | `OPEN=1`, `DATA=2`, `CLOSE=3`, `ERROR=4`, `PING=5`, `PONG=6` |
| `stream_id` | `u32` | Client-assigned stream ID (`0` reserved for connection-level messages) |
| `length` | `u32` | Payload length |
| `payload` | `bytes[length]` | Message payload |

#### UDP relay framing v1/v2 (WebRTC DataChannel `udp` and WebSocket `/udp`)

Used by `proxy/webrtc-udp-relay` for UDP proxying:

- **WebRTC DataChannel label:** `udp` (`ordered=false`, `maxRetransmits=0`)
- **WebSocket fallback:** `GET /udp` (message-oriented; one datagram frame per WS binary message)

Spec and implementations:

- **Spec:** [`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md)
- **Reference implementations:**
  - Go: `proxy/webrtc-udp-relay/internal/udpproto/*`
  - Browser: `apps/web/src/shared/udpRelayProtocol.ts`

Version detection rules (from `PROTOCOL.md`):

- If `len(frame) >= 2` and the first two bytes are `0xA2 0x02`, parse as **v2**.
- Otherwise, parse as **v1** (legacy, IPv4-only).

Negotiation summary:

- **IPv6 requires v2.**
- For IPv4, the relay may send v1 or v2 back to the client; relays typically only emit v2 after the
  client has demonstrated v2 support (to avoid breaking v1-only clients).

Semantic note: `/udp` over WebSocket uses the same framing, but WebSockets are **reliable and
ordered**, so it cannot perfectly emulate UDP loss/reordering semantics. Treat it as a fallback or
debug path.

#### `aero-l2-tunnel-v1` (Option C: L2 tunnel)

Used by the production L2 tunnel:

- **WebSocket:** `GET /l2` (subprotocol `aero-l2-tunnel-v1`)
- **WebSocket legacy alias:** `GET /eth` (subprotocol `aero-l2-tunnel-v1`)
- **WebRTC:** DataChannel label `l2` (must be fully reliable and ordered; `ordered = true`)

See: [`../specs/l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md)

### Deprecations / historical modules (avoid confusion)

- `server/` is the legacy monolith backend. New backend work should target:
  - `services/gateway` (TCP proxy + DoH + UDP relay token minting)
  - `proxy/webrtc-udp-relay` (WebRTC UDP relay + `/udp` fallback + `l2` bridge)
  - `crates/aero-l2-proxy` (Option C user-space stack / NAT)
  - `crates/aero-storage-server` (storage service; part of the backend split)
- `tools/aero-gateway-rs` is a **legacy/diagnostic** Rust gateway prototype that only implements the
  historical `/tcp?target=<host>:<port>` tunnel (plus a small admin/capture surface). It is not
  production-hardened and is intentionally excluded from the default Rust workspace build/test
  surface. The canonical gateway is `services/gateway` (Node/TypeScript).
- Guest network stack implementations exist in multiple places:
  - `crates/aero-net-stack` is the canonical Phase 0 in-browser stack
    (the native runtime used it directly).
  - Historical implementations have been retired/removed to reduce CI surface:
    - `crates/aero-net` (Tokio-era experiment; no longer part of the Rust workspace)
  - The recommended production path is Option C (L2 tunnel) as described above.
- Dev/prototype UDP behavior:
  - `services/net-proxy/` supports a legacy per-target UDP mode (`/udp?host=...&port=...`) that forwards raw
    UDP payload bytes. Prefer the multiplexed v1/v2 datagram framing (`/udp` with no target params)
    or WebRTC when building new integrations.

---

### Snapshot/Restore (Save States)

Networking state spans multiple layers, and **not all of it is bit-restorable**:

1. **Guest-visible NIC device model** (e.g. E1000 / virtio-net): registers + DMA rings.
2. **Host-side networking backend/stack**:
   - Phase 0 / fallback: in-browser `aero-net-stack` (DHCP/DNS/NAT + TCP/UDP proxy bookkeeping).
   - Production: L2 tunnel forwarder (pure frame pipe to a proxy that runs the TCP/IP stack).
3. **Host transports** (WebSocket/WebRTC objects) and **proxy-side sockets**.

In the VM snapshot file, networking is represented as two `DEVICES` entries:

| Layer | Outer `DeviceId` | Web/WASM `kind` string | What it covers |
|---|---|---|---|
| NIC (E1000) | `DeviceId::E1000` (`19`) | `net.e1000` | Guest-visible NIC state: registers, descriptor rings, pending RX/TX bookkeeping. |
| Net stack/backend | `DeviceId::NET_STACK` (`20`) | `net.stack` | User-space stack / DHCP/DNS cache + proxy bookkeeping. |

Device snapshot IDs and the stable `kind` strings used by the web runtime are listed in [`../specs/snapshot-format.md`](../specs/snapshot-format.md) (e.g. `net.e1000`, `net.stack`, and the forward-compatible `device.<id>` form).

#### Restore behavior (expected/required)

##### E1000 NIC (`net.e1000`)

- **Restored:** guest-visible NIC device model state (registers, ring base addresses, head/tail indices,
  interrupt mask/cause, MAC/EEPROM/PHY state, and the device model’s own pending RX/TX frame queues).
- **Not bit-restorable:** host-side network backends/transports and their buffers (e.g. live tunnel connections,
  shared `NET_TX`/`NET_RX` rings) are external state and are not part of VM snapshots.
  - Expect occasional packet loss immediately after restore if frames were in flight in host-side queues.
    Higher layers (TCP, application retries) must cope.

##### Net stack (`net.stack`, Phase 0 / fallback)

The in-browser `aero-net-stack` snapshots only the minimal dynamic state required for reasonable resume semantics, but it cannot bit-restore host-side proxy transports.

**Policy on restore:** **Drop.** On restore, all host-side proxy transports are treated as reset/closed.

- **Restored (best-effort):** config-independent bookkeeping like the learned guest MAC, an “IP assigned” flag, next-id counters, and a bounded DNS cache.
- **Dropped on restore (by design):**
  - in-flight DNS resolutions (`pending_dns`)
  - all active TCP connections (`tcp`)
  These are dropped because the host transport (WebSocket/WebRTC/TCP) is not bit-restorable and preserving them would imply reconnection logic that cannot provide correct guest TCP semantics.
- **Active TCP proxy connections are not bit-restorable.** Browser WebSocket objects cannot be serialized, and the proxy’s upstream sockets have independent state.
- Guest TCP connections that were mid-flight will break (RST/timeout) and must be re-established by the guest/application.
- UDP is connectionless; any in-flight datagrams may be dropped, but subsequent sends work once the stack is running again.

Implementation note: the stack also supports an explicit best-effort reconnect policy (`TcpRestorePolicy::Reconnect`), which restores only connection IDs/endpoints and attempts to re-open proxy tunnels. This mode cannot guarantee correct TCP semantics (and is expected to fail for many real-world protocols such as TLS). Canonical VM snapshot restore uses the deterministic **Drop** policy.

##### L2 tunnel (production Option C)

The L2 tunnel is a host transport (WebSocket/WebRTC DataChannel) carrying raw Ethernet frames; the connection itself is **not bit-restorable**.

- On resume, the tunnel reconnects **best-effort** (normal reconnect/backoff policy).
- The shared `NET_TX` / `NET_RX` rings are **cleared** during snapshot pause (before save/restore) to avoid replaying stale frames into a restored guest.

#### Snapshot ordering requirements (web runtime)

To avoid capturing partially-processed network traffic (and to ensure restored device state does not see stale ring contents):

1. **Pause CPU + I/O** (stop guest execution + device emulation).
2. **Then pause NET** (stop frame forwarding) and clear transient host traffic:
   - close/stop the tunnel transport (WebSocket/WebRTC),
   - drain/reset the shared `NET_TX`/`NET_RX` rings **only after** all producers/consumers are paused (to avoid races and stale replay).
3. Save/restore snapshot bytes.
4. **Resume CPU + I/O**, then resume NET; the net worker reconnects best-effort.

#### L2 tunnel forwarder (Option C) pause/drain policy

When using the production L2 tunnel path (the L2 tunnel networking decision), the browser runtime includes a dedicated network worker that forwards raw Ethernet frames between shared rings (`NET_TX`/`NET_RX`) and the tunnel transport (WebSocket/DataChannel).

For a consistent snapshot:

- The tunnel forwarder should be **paused/drained** during snapshot (stop reading/writing frames so the IO worker’s NIC state cannot race with in-flight tunnel activity).
- On resume/restore, the forwarder should be **restarted** and establish a fresh tunnel connection.

This is the same fundamental constraint as TCP proxy sockets: the transport itself is a host resource and must be treated as reset on restore.

Web runtime note: the current worker runtime implements this by having the net worker participate in the snapshot control protocol (`vm.snapshot.pause` / `vm.snapshot.resume`) and stop/drain the forwarder while paused.

### E1000 NIC Emulation

#### Register Interface

```rust
pub struct E1000Device {
    // Control registers
    ctrl: u32,           // Device Control
    status: u32,         // Device Status
    eecd: u32,           // EEPROM Control
    eerd: u32,           // EEPROM Read
    fla: u32,            // Flash Access
    ctrl_ext: u32,       // Extended Device Control
    mdic: u32,           // MDI Control
    
    // Interrupt registers
    icr: u32,            // Interrupt Cause Read
    itr: u32,            // Interrupt Throttling
    ics: u32,            // Interrupt Cause Set
    ims: u32,            // Interrupt Mask Set
    imc: u32,            // Interrupt Mask Clear
    
    // Receive registers
    rctl: u32,           // Receive Control
    rdbal: u32,          // RX Descriptor Base Low
    rdbah: u32,          // RX Descriptor Base High
    rdlen: u32,          // RX Descriptor Length
    rdh: u32,            // RX Descriptor Head
    rdt: u32,            // RX Descriptor Tail
    
    // Transmit registers
    tctl: u32,           // Transmit Control
    tdbal: u32,          // TX Descriptor Base Low
    tdbah: u32,          // TX Descriptor Base High
    tdlen: u32,          // TX Descriptor Length
    tdh: u32,            // TX Descriptor Head
    tdt: u32,            // TX Descriptor Tail
    
    // MAC address
    mac_addr: [u8; 6],
    
    // Packet buffers
    rx_queue: VecDeque<Vec<u8>>,
    tx_queue: VecDeque<Vec<u8>>,
    
    // EEPROM
    eeprom: [u16; 64],
}

#[repr(C)]
pub struct E1000RxDescriptor {
    buffer_addr: u64,    // Buffer address
    length: u16,         // Length
    checksum: u16,       // Packet checksum
    status: u8,          // Status
    errors: u8,          // Errors
    special: u16,        // Special (VLAN)
}

#[repr(C)]
pub struct E1000TxDescriptor {
    buffer_addr: u64,    // Buffer address
    length: u16,         // Length
    cso: u8,             // Checksum Offset
    cmd: u8,             // Command
    status: u8,          // Status
    css: u8,             // Checksum Start
    special: u16,        // Special (VLAN)
}
```

#### Packet Processing

```rust
impl E1000Device {
    pub fn process_tx(&mut self, memory: &MemoryBus) {
        // Check if transmit is enabled
        if self.tctl & E1000_TCTL_EN == 0 {
            return;
        }
        
        let desc_base = ((self.tdbah as u64) << 32) | (self.tdbal as u64);
        let desc_count = self.tdlen / 16;
        
        // Process descriptors from head to tail
        while self.tdh != self.tdt {
            let desc_addr = desc_base + (self.tdh as u64) * 16;
            let mut desc: E1000TxDescriptor = memory.read_struct(desc_addr);
            
            // Read packet data
            let packet_data = memory.read_bytes(desc.buffer_addr, desc.length as usize);
            
            // Queue for transmission
            self.tx_queue.push_back(packet_data);
            
            // Update descriptor status
            desc.status |= E1000_TXD_STAT_DD;  // Descriptor Done
            memory.write_struct(desc_addr, &desc);
            
            // Advance head
            self.tdh = (self.tdh + 1) % desc_count;
        }
        
        // Raise TX interrupt if enabled
        if self.ims & E1000_ICR_TXDW != 0 {
            self.icr |= E1000_ICR_TXDW;
            self.raise_irq();
        }
    }
    
    pub fn receive_packet(&mut self, packet: &[u8], memory: &mut MemoryBus) {
        // Check if receive is enabled
        if self.rctl & E1000_RCTL_EN == 0 {
            return;
        }
        
        let desc_base = ((self.rdbah as u64) << 32) | (self.rdbal as u64);
        let desc_count = self.rdlen / 16;
        
        // Get next available descriptor
        let next_desc = (self.rdh + 1) % desc_count;
        if next_desc == self.rdt {
            // No descriptors available - drop packet
            return;
        }
        
        let desc_addr = desc_base + (self.rdh as u64) * 16;
        let mut desc: E1000RxDescriptor = memory.read_struct(desc_addr);
        
        // Copy packet to descriptor buffer
        memory.write_bytes(desc.buffer_addr, packet);
        
        // Update descriptor
        desc.length = packet.len() as u16;
        desc.status = E1000_RXD_STAT_DD | E1000_RXD_STAT_EOP;
        desc.errors = 0;
        memory.write_struct(desc_addr, &desc);
        
        // Advance head
        self.rdh = next_desc;
        
        // Raise RX interrupt if enabled
        if self.ims & E1000_ICR_RXT0 != 0 {
            self.icr |= E1000_ICR_RXT0;
            self.raise_irq();
        }
    }
}
```

---

### User-space Network Stack

#### Ethernet Frame Processing

```rust
pub struct NetworkStack {
    mac_addr: [u8; 6],
    ip_addr: Ipv4Addr,
    gateway: Ipv4Addr,
    netmask: Ipv4Addr,
    dns_servers: Vec<Ipv4Addr>,
    
    // ARP table
    arp_table: HashMap<Ipv4Addr, [u8; 6]>,
    
    // TCP connections (for NAT tracking)
    tcp_connections: HashMap<(u16, Ipv4Addr, u16), TcpConnection>,
    
    // UDP bindings
    udp_bindings: HashMap<u16, UdpBinding>,
}

impl NetworkStack {
    pub fn process_outgoing(&mut self, frame: &[u8]) -> Option<NetworkAction> {
        // Parse Ethernet header
        if frame.len() < 14 {
            return None;
        }
        
        let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        let payload = &frame[14..];
        
        match ethertype {
            0x0800 => self.process_ipv4(payload),  // IPv4
            0x0806 => self.process_arp(payload),   // ARP
            0x86DD => self.process_ipv6(payload),  // IPv6
            _ => None,
        }
    }
    
    fn process_ipv4(&mut self, packet: &[u8]) -> Option<NetworkAction> {
        if packet.len() < 20 {
            return None;
        }
        
        let ihl = (packet[0] & 0x0F) as usize * 4;
        let protocol = packet[9];
        let src_ip = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
        let dst_ip = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
        
        let payload = &packet[ihl..];
        
        match protocol {
            6 => self.process_tcp(src_ip, dst_ip, payload),   // TCP
            17 => self.process_udp(src_ip, dst_ip, payload),  // UDP
            1 => self.process_icmp(src_ip, dst_ip, payload),  // ICMP
            _ => None,
        }
    }
    
    fn process_tcp(&mut self, src_ip: Ipv4Addr, dst_ip: Ipv4Addr, segment: &[u8]) -> Option<NetworkAction> {
        if segment.len() < 20 {
            return None;
        }
        
        let src_port = u16::from_be_bytes([segment[0], segment[1]]);
        let dst_port = u16::from_be_bytes([segment[2], segment[3]]);
        let flags = segment[13];
        
        let conn_key = (src_port, dst_ip, dst_port);
        
        if flags & TCP_SYN != 0 && flags & TCP_ACK == 0 {
            // New connection - create WebSocket
            Some(NetworkAction::ConnectTcp {
                local_port: src_port,
                remote_ip: dst_ip,
                remote_port: dst_port,
            })
        } else if let Some(conn) = self.tcp_connections.get_mut(&conn_key) {
            // Existing connection - forward data
            let data_offset = ((segment[12] >> 4) as usize) * 4;
            let payload = &segment[data_offset..];
            
            if !payload.is_empty() {
                Some(NetworkAction::SendTcp {
                    connection_id: conn.id,
                    data: payload.to_vec(),
                })
            } else {
                None
            }
        } else {
            None
        }
    }
}
```

#### DHCP Client

```rust
pub struct DhcpClient {
    state: DhcpState,
    transaction_id: u32,
    offered_ip: Option<Ipv4Addr>,
    server_ip: Option<Ipv4Addr>,
    lease_time: u32,
}

enum DhcpState {
    Init,
    Selecting,
    Requesting,
    Bound,
    Renewing,
    Rebinding,
}

impl DhcpClient {
    pub fn start_discovery(&mut self) -> Vec<u8> {
        self.state = DhcpState::Selecting;
        self.transaction_id = rand::random();
        
        self.build_dhcp_discover()
    }
    
    pub fn handle_response(&mut self, packet: &[u8]) -> Option<NetworkConfig> {
        let dhcp = self.parse_dhcp(packet)?;
        
        match self.state {
            DhcpState::Selecting if dhcp.message_type == DHCP_OFFER => {
                self.offered_ip = Some(dhcp.your_ip);
                self.server_ip = Some(dhcp.server_id);
                self.state = DhcpState::Requesting;
                // Send DHCP Request
                None
            }
            DhcpState::Requesting if dhcp.message_type == DHCP_ACK => {
                self.state = DhcpState::Bound;
                self.lease_time = dhcp.lease_time;
                
                Some(NetworkConfig {
                    ip_address: dhcp.your_ip,
                    subnet_mask: dhcp.subnet_mask,
                    gateway: dhcp.router,
                    dns_servers: dhcp.dns_servers,
                })
            }
            _ => None,
        }
    }
    
    fn build_dhcp_discover(&self) -> Vec<u8> {
        let mut packet = vec![0u8; 548];
        
        packet[0] = 1;  // BOOTREQUEST
        packet[1] = 1;  // Ethernet
        packet[2] = 6;  // Hardware address length
        packet[3] = 0;  // Hops
        
        // Transaction ID
        packet[4..8].copy_from_slice(&self.transaction_id.to_be_bytes());
        
        // Flags: broadcast
        packet[10] = 0x80;
        packet[11] = 0x00;
        
        // Client hardware address
        packet[28..34].copy_from_slice(&self.mac_addr);
        
        // Magic cookie
        packet[236..240].copy_from_slice(&[99, 130, 83, 99]);
        
        // Options
        let mut opt_offset = 240;
        
        // DHCP Message Type = Discover
        packet[opt_offset] = 53;
        packet[opt_offset + 1] = 1;
        packet[opt_offset + 2] = 1;  // Discover
        opt_offset += 3;
        
        // End
        packet[opt_offset] = 255;
        
        packet
    }
}
```

---

### WebSocket TCP Proxy

All outbound TCP connections from Aero are bridged through the **Aero Gateway** backend (see `services/gateway`). For the authoritative contract, see:

- [Aero Gateway API](../specs/gateway-api.md)
- [Aero Gateway OpenAPI](../../services/gateway/openapi.yaml)

#### Reference implementation: `services/net-proxy/`

This repository includes a standalone WebSocket → TCP/UDP relay service in [`services/net-proxy/`](../../services/net-proxy/). It is suitable for:

- local development (run alongside `vite dev`)
- E2E testing (no public internet required)

For production deployments, prefer `services/gateway` (TCP+DNS) and
`proxy/webrtc-udp-relay` (UDP).

To run in trusted local development mode (allows `127.0.0.1`, RFC1918, etc):

```bash
pnpm install --frozen-lockfile
AERO_PROXY_OPEN=1 pnpm -C services/net-proxy run dev
```

Health check:

```bash
curl http://127.0.0.1:8081/healthz
```

DoH endpoints (DNS-over-HTTPS):

- `GET|POST /dns-query` (RFC 8484; `application/dns-message`)
- `GET /dns-json` (`application/dns-json`, Cloudflare-DNS-JSON compatible)

See [`services/net-proxy/README.md`](../decisions/README.md) for browser configuration, CORS notes, and curl examples.

UDP relay modes:

- `WS /udp` (no `host`/`port`/`target` query params): multiplexed UDP relay framing (v1/v2 datagrams) per [`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md).
- `WS /udp?v=1&host=<host>&port=<port>` (or `target=<host>:<port>`): legacy per-target UDP relay (raw UDP payload bytes).

The simplest approach is one WebSocket per TCP connection (`/tcp`). For high
connection counts (thousands of concurrent guest sockets), use multiplexing
many TCP streams over a single WebSocket connection (`GET /tcp-mux`,
subprotocol `aero-tcp-mux-v1`). Both the production gateway (`services/gateway`)
and the local dev relay (`services/net-proxy/`) expose `/tcp-mux`. See
[`../specs/gateway-api.md`](../specs/gateway-api.md) for the
wire protocol.

#### Client-side (Browser)

```rust
pub struct TcpProxy {
    connections: HashMap<u32, WebSocket>,
    next_id: u32,
    proxy_url: String,
}

impl TcpProxy {
    pub async fn connect(&mut self, remote_ip: Ipv4Addr, remote_port: u16) -> Result<u32> {
        let id = self.next_id;
        self.next_id += 1;
        
        // Canonical Aero Gateway endpoint format:
        //   /tcp?v=1&host=<hostname-or-ip>&port=<port>
        //
        // For compatibility with older clients, the gateway may also accept
        // a legacy `target=<host>:<port>` query parameter, but new clients
        // should always use `host` + `port`.
        let url = format!(
            "{}/tcp?v=1&host={}&port={}",
             self.proxy_url,
             remote_ip,
             remote_port
        );
        
        let ws = WebSocket::new(&url)?;
        
        ws.set_binary_type(BinaryType::Arraybuffer);
        
        let (tx, rx) = channel();
        
        ws.set_onmessage(move |event| {
            if let Some(data) = event.data().as_array_buffer() {
                let bytes = Uint8Array::new(&data).to_vec();
                tx.send(bytes).ok();
            }
        });
        
        self.connections.insert(id, ws);
        
        Ok(id)
    }
    
    pub fn send(&self, connection_id: u32, data: &[u8]) -> Result<()> {
        if let Some(ws) = self.connections.get(&connection_id) {
            ws.send_with_u8_array(data)?;
        }
        Ok(())
    }
    
    pub fn close(&mut self, connection_id: u32) {
        if let Some(ws) = self.connections.remove(&connection_id) {
            ws.close().ok();
        }
    }
}
```

#### Server-side (Aero Gateway)

The TCP relay is a security-critical component (SSRF, port scanning, abuse). Do not deploy an ad-hoc “minimal TCP proxy” in production.

Use the maintained gateway implementation in `services/gateway` and follow:

- [Aero Gateway API](../specs/gateway-api.md)
- [Aero Gateway OpenAPI](../../services/gateway/openapi.yaml)

#### Security

The gateway must enforce (at minimum):

- **Origin allowlist**: validate `Origin` (WebSockets) and apply strict CORS (HTTP endpoints).
- **Authentication**: require a cookie-backed session or an explicit token (including a WebSocket-compatible mechanism).
- **Blocked destinations**: deny private/loopback/link-local/multicast ranges, and re-check post-DNS resolution to prevent DNS rebinding.
- **Port allowlist**: only allow configured outbound ports (deny-by-default).
- **Rate limiting & quotas**: per-user/IP limits on connection attempts, concurrent sockets, and bytes transferred.

#### Scaling: TCP multiplexing (`/tcp-mux`)

For workloads that need many concurrent TCP connections, the gateway can optionally expose a multiplexed WebSocket endpoint (`/tcp-mux`) that carries many logical TCP streams over a single socket. This reduces WebSocket overhead and avoids browser connection limits. See the [Aero Gateway API](../specs/gateway-api.md) for details.

##### TCP Egress Policy (Recommended for Public Deployments)

When exposing the TCP proxy endpoints (`/tcp` and `/tcp-mux`) publicly, it's recommended to restrict outbound
connections to a safe subset of domains to reduce abuse risk.

Environment variables:

- `TCP_ALLOWED_HOSTS` (default: empty / allow-all)
  - Comma-separated hostname patterns.
  - Supports exact matches (`example.com`) and wildcard subdomain matches
    (`*.example.com`).
  - If non-empty, the target **must** match at least one pattern.
- `TCP_BLOCKED_HOSTS` (default: empty)
  - Comma-separated hostname patterns using the same syntax.
  - Always enforced; deny overrides allow.
- `TCP_REQUIRE_DNS_NAME` (default: `0`)
  - When set to `1`, disallows IP-literal targets entirely (forces DNS names).

Notes:

- Hostnames are normalized before matching (lowercased, IDNA/punycode for
  international domains). Invalid hostnames are rejected.
- Hostname allow/deny decisions are applied **before** DNS resolution.
- Private/reserved IP blocking still applies after DNS resolution. If a hostname
  resolves to multiple IPs, the proxy connects only to the **first** allowed
  public IP.

For local development/testing of the `/tcp-mux` framing protocol (`aero-tcp-mux-v1`):

- **Production (Aero Gateway):** canonical framing implemented by `services/gateway`.
- **Local development relay (recommended):** `services/net-proxy/` exposes `/tcp-mux` and uses `AERO_PROXY_OPEN` / `AERO_PROXY_ALLOW` for per-stream policy.
- **Dev relay (standalone):** `tools/net-proxy-server/` speaks the same framing, but uses `?token=` auth (not gateway cookie sessions).
- **Browser client:** `apps/web/src/net/tcpMuxProxy.ts`.
- **TcpProxyEvent adapter:** `apps/web/src/net/tcpProxy.ts` (`WebSocketTcpProxyMuxClient`) exposes the mux client behind the same event-sink interface as the legacy one-WebSocket-per-connection `WebSocketTcpProxyClient`.

---

### WebRTC UDP Proxy

#### Security warning (server-side relay)

Running a UDP relay makes your server a **network egress point**. If it is reachable by untrusted clients and forwards UDP to arbitrary destinations, it can be abused as an **open proxy / SSRF primitive** (internal network scanning, hitting link-local services, etc.).

**Recommendation:** default to **local-only** deployment (bind on `127.0.0.1` / behind auth) and enforce an explicit destination policy (CIDR + port allowlists) in production.

The WebRTC UDP relay DataChannel framing and signaling schema are specified in
[`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md).

The same relay also exposes `GET /udp` as a WebSocket fallback using the same v1/v2 datagram framing.

**Semantic note:** WebSockets are reliable and ordered, so `/udp` cannot preserve UDP loss/reordering
semantics and may introduce head-of-line blocking. Treat it as a fallback/debug path, not the
real-time/low-latency path.

#### Browser integration: gateway-minted relay credentials

The browser should **not** embed long-lived relay secrets. Instead, it obtains a short-lived relay token from the Aero Gateway:

1. Call `POST /session` on the gateway with `credentials: "include"`.
2. Read the optional `udpRelay` field from the JSON response (only present when `UDP_RELAY_BASE_URL` is configured).
3. Use `udpRelay.baseUrl` + `udpRelay.endpoints.*` to build relay URLs.

Note: `udpRelay.baseUrl` may be configured as either an **HTTP(S)** or **WebSocket (WS/S)** URL.
Clients must normalize schemes depending on the endpoint transport:

- HTTP endpoints (`webrtcOffer`, `webrtcIce`): use `http(s)://` (`ws://` → `http://`, `wss://` → `https://`).
- WebSocket endpoints (`webrtcSignal`, `udp`): use `ws(s)://` (`http://` → `ws://`, `https://` → `wss://`).

In particular, browser `fetch()` does **not** support `ws://` / `wss://` URLs.

Endpoint meanings:

- `webrtcSignal`: WebSocket signaling (trickle ICE): `ws(s)://…/webrtc/signal`
- `webrtcOffer`: HTTP signaling fallback (non-trickle ICE): `http(s)://…/webrtc/offer`
- `webrtcIce`: HTTP ICE server discovery: `http(s)://…/webrtc/ice`
- `udp`: WebSocket UDP fallback (non-WebRTC): `ws(s)://…/udp`

When `udpRelay.authMode` is:

- `none`: connect directly.
- `api_key` / `jwt`: clients must authenticate to the relay service. Credential delivery options:
  - **Query string** (simple but less preferred; can leak into logs/history):
    - `api_key`: `?apiKey=<token>` (or `?token=<token>` for compatibility)
    - `jwt`: `?token=<token>` (or `?apiKey=<token>` for compatibility)
  - **First WebSocket message** (recommended for browser clients): send a JSON text auth message, then proceed:
    - `{ "type":"auth", "apiKey":"<token>" }` or `{ "type":"auth", "token":"<token>" }`
  - **Upgrade request headers** (best for non-browser clients): use standard header carriers (`Authorization` / `X-API-Key`), per [`proxy/webrtc-udp-relay/PROTOCOL.md`](../../proxy/webrtc-udp-relay/PROTOCOL.md).

Some deployments additionally expose `POST /udp-relay/token` on the gateway to refresh the short-lived relay token without re-running the full session bootstrap.

```rust
pub struct UdpProxy {
    peer_connection: RtcPeerConnection,
    data_channel: RtcDataChannel,
    bindings: HashMap<u16, mpsc::Sender<UdpPacket>>,
}

impl UdpProxy {
    pub async fn initialize(signaling_url: &str) -> Result<Self> {
        let config = RtcConfiguration {
            ice_servers: vec![
                RtcIceServer {
                    urls: vec!["stun:stun.l.google.com:19302".to_string()],
                },
            ],
        };
        
        let pc = RtcPeerConnection::new(&config)?;
        
        // Create data channel for UDP.
        //
        // Note: this DataChannel config is for the UDP relay, where best-effort/lossy semantics are
        // acceptable. If you carry the **L2 tunnel** over WebRTC, the channel MUST be reliable and
        // ordered (do NOT set `maxRetransmits`/`maxPacketLifeTime`, and do not set `ordered: false`).
        // See the L2 tunnel networking decision and `../specs/l2-tunnel-protocol.md`.
        let dc = pc.create_data_channel("udp", &RtcDataChannelInit {
            ordered: false,
            max_retransmits: Some(0),
        })?;
        
        // Signaling to establish connection with proxy server
        let offer = pc.create_offer().await?;
        pc.set_local_description(&offer).await?;
        
        // Send offer to signaling server
        let answer = Self::signal(signaling_url, &offer).await?;
        pc.set_remote_description(&answer).await?;
        
        Ok(Self {
            peer_connection: pc,
            data_channel: dc,
            bindings: HashMap::new(),
        })
    }
    
    pub fn send(&self, guest_port: u16, remote_ip: Ipv4Addr, remote_port: u16, data: &[u8]) -> Result<()> {
        // Create a UDP relay frame. v1 is IPv4-only; v2 is required for IPv6.
        // See `proxy/webrtc-udp-relay/PROTOCOL.md` for details.
        let mut packet = Vec::with_capacity(8 + data.len());
        packet.extend_from_slice(&guest_port.to_be_bytes());
        packet.extend_from_slice(&remote_ip.octets());
        packet.extend_from_slice(&remote_port.to_be_bytes());
        packet.extend_from_slice(data);
        
        self.data_channel.send(&packet)?;
        Ok(())
    }
}
```

**IPv6 note:** The 8-byte header shown above is the legacy **v1** framing and is
IPv4-only. Relaying to IPv6 destinations (and receiving IPv6 datagrams) requires
the versioned **v2** framing defined in `proxy/webrtc-udp-relay/PROTOCOL.md`.
 
 ---

### DNS Resolution

In browser environments, DNS lookups should go through the gateway’s **first-party** DNS-over-HTTPS endpoints:

- `/dns-query` (RFC 8484 DoH, `application/dns-message`)
- `/dns-json` (optional JSON convenience endpoint)

For simple browser/WASM clients that do not want to parse DNS wire format, `/dns-json` is the easiest integration:

```ts
const res = await fetch(`/dns-json?name=${hostname}&type=A`, {
  headers: { accept: 'application/dns-json' },
});
const json = await res.json();
const ip = json.Answer?.[0]?.data; // e.g. "93.184.216.34"
```

```rust
pub struct DnsResolver {
    gateway_url: String,
    dns_servers: Vec<Ipv4Addr>,
    cache: HashMap<String, DnsCacheEntry>,
}

impl DnsResolver {
    pub fn resolve(&mut self, hostname: &str) -> Option<Ipv4Addr> {
        // Check cache
        if let Some(entry) = self.cache.get(hostname) {
            if entry.expires > Instant::now() {
                return Some(entry.address);
            }
        }
        
        // Build DNS query
        let query = self.build_query(hostname);
        
        // In browser context, we need to proxy DNS through the server
        // or use DNS-over-HTTPS
        None
    }
    
    pub async fn resolve_doh(&self, hostname: &str) -> Result<Ipv4Addr> {
        // Use Aero Gateway DNS-over-HTTPS (first-party).
        // Canonical DoH: `/dns-query` (RFC 8484, `application/dns-message`).
        //
        // Note: some deployments may also expose `/dns-json` for simple lookups,
        // but clients should not depend on it (it is not required for DoH).
        let query = self.build_query_message(hostname, RecordType::A)?;
        let dns = base64url_encode(&query);
        let url = format!("{}/dns-query?dns={}", self.gateway_url, dns);
        
        let response = fetch(&url, FetchOptions {
            headers: vec![("Accept".to_string(), "application/dns-message".to_string())],
        }).await?;
        
        let bytes = response.bytes().await?;
        let ip = parse_first_a_record(&bytes)?;
        Ok(ip)
    }
}
```

---

### Virtio-net (Paravirtualized)

> For the exact Windows 7 driver ↔ Aero device-model interoperability contract (PCI transport, virtqueue rules, and virtio-net requirements), see:  
> [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md)  
>
> For the split-ring virtqueue implementation algorithms used by Windows 7 KMDF virtio drivers, see:  
> [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md)

```rust
pub struct VirtioNetDevice {
    // Virtio common
    device_features: u64,
    driver_features: u64,
    
    // Device config
    mac: [u8; 6],
    status: u16,
    max_virtqueue_pairs: u16,
    
    // Virtqueues
    rx_vq: Virtqueue,
    tx_vq: Virtqueue,
    ctrl_vq: Option<Virtqueue>,
}

impl VirtioNetDevice {
    pub fn process_tx(&mut self, memory: &MemoryBus, network: &mut NetworkStack) {
        while let Some(desc_chain) = self.tx_vq.pop_available(memory) {
            // First descriptor is virtio_net_hdr
            let header_addr = desc_chain[0].addr;
            let header: VirtioNetHeader = memory.read_struct(header_addr);
            
            // Remaining descriptors are packet data
            let mut packet = Vec::new();
            for desc in &desc_chain[1..] {
                let data = memory.read_bytes(desc.addr, desc.len as usize);
                packet.extend_from_slice(&data);
            }
            
            // Process packet
            if let Some(action) = network.process_outgoing(&packet) {
                // Handle network action
            }
            
            // Return descriptor
            self.tx_vq.push_used(desc_chain.head_id, 0, memory);
        }
        
        self.maybe_notify_guest();
    }
    
    pub fn receive_packet(&mut self, packet: &[u8], memory: &mut MemoryBus) {
        if let Some(desc_chain) = self.rx_vq.pop_available(memory) {
            // Write virtio_net_hdr to first descriptor
            let header = VirtioNetHeader::default();
            memory.write_struct(desc_chain[0].addr, &header);
            
            // Write packet to second descriptor
            memory.write_bytes(desc_chain[1].addr, packet);
            
            // Return with total length
            let total_len = std::mem::size_of::<VirtioNetHeader>() + packet.len();
            self.rx_vq.push_used(desc_chain.head_id, total_len as u32, memory);
            
            self.maybe_notify_guest();
        }
    }
}
```

---

### Network Tracing (PCAP/PCAPNG Export)

When debugging network bring-up, it is often necessary to see the exact guest Ethernet frames (TX from the guest NIC and RX to the guest NIC), as well as emulator-generated traffic (e.g. TCP proxy I/O) for correlation.

The network stack should support an *optional* tracing component that:

- Captures **raw Ethernet frames** with timestamps (TX/RX) suitable for Wireshark.
- Optionally captures **post-NAT / proxy bytes** on a separate pseudo-interface for correlation.
- Can be enabled at runtime in dev builds, but is **off by default**.

#### Implementation Notes (Repo)

The canonical Rust implementation lives in `crates/aero-net-trace/` and provides:

- `NetTracer` + `NetTraceConfig` for capturing frames and exporting PCAPNG.
- `TracedNetworkStack` wrapper that records:
  - Guest TX/RX Ethernet frames at the stack boundary
  - TCP proxy payloads (`ProxyAction::TcpSend` / `ProxyEvent::TcpData`) on a separate pseudo-interface.

Rust capture format notes (PCAPNG):

- Ethernet frames are written on a single interface named `guest-eth0`; packet direction is encoded
  via the Enhanced Packet Block `epb_flags` option (`1` = inbound, `2` = outbound).
- Proxy payload capture is opt-in by default (requires enabling `capture_tcp_proxy` / `capture_udp_proxy` in `NetTraceConfig`).
- If proxy payload records are present, additional pseudo-interfaces may be emitted:
  - `tcp-proxy` (`LINKTYPE_USER0` / 147) containing pseudo-packets with an `ATCP` header.
  - `udp-proxy` (`LINKTYPE_USER1` / 148) containing pseudo-packets with an `AUDP` header.

#### Web runtime tracing (browser / worker runtime)

When running the **worker-based web runtime** with the **Option C L2 tunnel**, you can capture the
raw Ethernet frames being forwarded between the guest and the tunnel client and export them as a
`.pcapng` file (openable in Wireshark).

Note: this is implemented in the web runtime (TypeScript) and is separate from the Rust-side
`crates/aero-net-trace/` `NetTracer` described above; both export PCAPNG, but they run in
different runtimes and capture at different boundaries.

##### What it captures (L2 boundary)

The web tracing capture is taken at the **L2 forwarder boundary** in the browser:

- **Guest → tunnel:** frames drained from the guest-facing `NET_TX` ring before they are sent to the
  tunnel transport.
- **Tunnel → guest:** frames received from the tunnel transport before they are pushed into the
  guest-facing `NET_RX` ring.

In other words, it captures **guest↔tunnel Ethernet frames** right where the browser runtime acts as
an Ethernet “pipe” (the same conceptual boundary described in the L2 tunnel networking decision / Option C).

Capture format notes:

- Ethernet frames are written on a single interface named `guest-eth0`; packet direction is encoded
  via the Enhanced Packet Block `epb_flags` option (`1` = inbound, `2` = outbound).
- If proxy payload records are present, additional pseudo-interfaces may be emitted:
  - `tcp-proxy` (`LINKTYPE_USER0` / 147) containing pseudo-packets with an `ATCP` header.
  - `udp-proxy` (`LINKTYPE_USER1` / 148) containing pseudo-packets with an `AUDP` header.
- Note: in the worker-based web runtime, proxy payload capture is opt-in:
  - `NetTracer` defaults `captureTcpProxy=false` and `captureUdpProxy=false` (matching Rust).
  - The proxy pseudo-interfaces only appear if proxy capture is enabled *and* something explicitly
    calls `NetTracer.recordTcpProxy()` / `recordUdpProxy()`.
- Frames are recorded at the forwarder boundary (best-effort). In particular, the capture may
  include frames that were later dropped due to missing tunnel/backpressure, or because `NET_RX`
  was full.
- Timestamps are stored in nanoseconds (PCAPNG resolution `10^-9`), but in the current web runtime
  they are derived from `Date.now()` (millisecond wall-clock time), so do not expect
  sub-millisecond precision.

Relevant implementation files:

- UI surface: `apps/web/src/net/trace_ui.ts`
- `window.aero.netTrace` installer (bridges worker coordinator → global API): `apps/web/src/net/trace_backend.ts`
- Tunnel forwarder owner (where the capture boundary lives): `apps/web/src/workers/net.worker.ts`
- Forwarder capture hook semantics: `apps/web/src/net/l2TunnelForwarder.ts`
- Capture implementation + PCAPNG writer: `apps/web/src/net/net_tracer.ts`, `apps/web/src/net/pcapng.ts`
- Coordinator API + message types (worker runtime): `apps/web/src/runtime/coordinator.ts`, `apps/web/src/runtime/protocol.ts`
- Shared type definition for the global API: `shared/aero_api.ts` (`AeroNetTraceApi`)

##### UI workflow (“Network trace (PCAPNG)” panel)

In the repo’s browser host UI (repo-root Vite app; see `src/main.ts`), there is a panel titled
**“Network trace (PCAPNG)”**. The experimental `apps/web/` host UI (`apps/web/src/main.ts`) also exposes this panel.
It provides:

- **Enable/disable** tracing via a checkbox.
- (Optional) **Live stats** (captured bytes/records + drop counters) when the backend implements a stats API.
- **Clear capture** (when supported by the backend).
- **Download capture (PCAPNG)** to your local machine.
- **Download snapshot (PCAPNG)** (when supported): exports without clearing the in-memory buffer.
- **Save capture to OPFS** (Origin Private File System) at a configurable path (default
  `captures/aero-net-trace.pcapng`).

Tip: if you're debugging **early boot networking** (DHCP/DNS), enable tracing before starting the VM
so you don't miss the initial traffic.

Worker-runtime prerequisite: this tracing runs in the net worker and requires the worker-based web
runtime (SharedArrayBuffer/cross-origin isolation). If `SharedArrayBuffer` is unavailable, the net
worker (and thus tracing) will not run. See:
[`11-browser-apis.md`](web-host.md#cross-origin-isolation-coopcoep-deployment-requirements).

Disabling tracing stops recording new frames but does **not** automatically clear any frames already
buffered in memory (use **Clear capture** or download/export to reset).

Captures are kept **in-memory** until you export/save them. Reloading the page (or the net worker
crashing/restarting) will discard any buffered frames.

Downloading/saving a capture drains the current in-memory buffer, but does **not** automatically
disable tracing — if tracing is still enabled, new frames will continue to be recorded into a fresh
buffer.

If the tracing backend is not installed in the current build/runtime (e.g. `window.aero.netTrace`
is missing), the UI will surface an error when you try to enable/export.

If the net worker isn't running yet, captures may not record anything until it starts. Some hosts
explicitly require the net worker to be running before enabling/exporting. Others (notably the
default `window.aero.netTrace` backend installed by `apps/web/src/net/trace_backend.ts`) treat export/stats
as best-effort and return an **empty-but-valid** PCAPNG (and stub stats) until the worker is ready.

Most web hosts install the `window.aero.netTrace` backend by calling
`installNetTraceBackendOnAeroGlobal(...)` (see `apps/web/src/net/trace_backend.ts`).

If you are building a custom web host and want the automation API for DevTools or Playwright-style
scripts, ensure you call this after creating the worker coordinator:

```ts
import { installNetTraceBackendOnAeroGlobal } from "./net/trace_backend";

installNetTraceBackendOnAeroGlobal(workerCoordinator);
```

OPFS notes:

- OPFS is origin-scoped browser storage (`navigator.storage.getDirectory()`); it is convenient for
  keeping large captures without triggering a download prompt.
- OPFS may not be available in all browsers/contexts; the UI will error if unsupported.
- In Chromium-based browsers, you can inspect files written to OPFS via DevTools → **Application**
  → **Storage** → **Origin Private File System**.

##### Viewing the capture (Wireshark)

The downloaded/saved file is a standard **PCAPNG**. Open it in Wireshark and use normal display
filters. Common ones when debugging guest bring-up:

- `arp` (gateway discovery)
- `bootp` (DHCP)
- `dns` (DNS queries/responses)
- `tcp` / `udp` / `icmp`

Because the capture uses a single Ethernet interface (`guest-eth0`) with direction encoded via
`epb_flags`, you can view both inbound/outbound packets together in normal Wireshark flows. Use the packet
direction field plus “Follow Stream” and normal display filters as needed.

##### Automation API (`window.aero.netTrace`)

For browser automation and debugging from DevTools, the web runtime exposes an optional API at:

`window.aero.netTrace`

When present, it implements:

- `isEnabled(): boolean`
- `enable(): void`
- `disable(): void`
- `downloadPcapng(): Promise<Uint8Array>`
  - Draining export (clears the in-memory buffer after exporting).

Some builds may additionally expose:

- `clear(): void | Promise<void>` (drop buffered frames)
- `getStats(): unknown | Promise<unknown>` (implementation-defined counters such as buffered bytes/frames and drops)
  - When implemented in the web runtime, it typically returns:
    `{ enabled, records, bytes, droppedRecords, droppedBytes }` (same shape as `NetTracer.stats()` in `apps/web/src/net/net_tracer.ts`).
- `exportPcapng(): Promise<Uint8Array>` (non-draining snapshot export)

Worker-runtime note: in the worker-based runtime, these operations are implemented by sending
`net.trace.*` `postMessage()` commands to the net worker and receiving the `net.trace.pcapng`
response (`take_pcapng` drains; `export_pcapng` snapshots). See `apps/web/src/runtime/coordinator.ts`
(helper methods) and `apps/web/src/workers/net.worker.ts` (the handler + `NetTracer`).

Example:

```js
window.aero.netTrace.enable();
// ...reproduce the issue...
const bytes = await window.aero.netTrace.downloadPcapng();
console.log("pcapng bytes:", bytes.byteLength);
```

##### Capture size limits + sensitivity

PCAP/PCAPNG captures can be **large** and may contain **sensitive data** (credentials, cookies, DNS
queries, internal IPs, etc). Treat captures like secrets.

Both the Rust and web tracing implementations buffer captured packets **in-memory** and enforce a
hard size cap to prevent unbounded growth:

- **Rust (`crates/aero-net-trace/NetTracer`):**
  - `NetTraceConfig.max_bytes` defaults to **16 MiB** of captured payload bytes (not including PCAPNG overhead).
  - Once the cap is reached, new records are **dropped**; counters are available via `NetTracer.stats()`
    (`dropped_records` / `dropped_bytes`).
  - For sensitive environments, configure `NetTraceConfig.redactor` to reduce payload capture (e.g.
    `TruncateRedactor` or `HeadersOnlyRedactor` in `aero_net_trace`).
- **Web (`apps/web/src/net/net_tracer.ts`):**
  - Defaults to **16 MiB** of captured payload bytes (`maxBytes`) per net worker.
  - When the cap is reached, new frames are **dropped**; counters are available via `NetTracer.stats()`
    (`droppedRecords` / `droppedBytes`) when surfaced by the host/backend.

Export/clear semantics (important for interpreting drop counters):

- **Draining export**:
  - Rust: `NetTracer.take_pcapng()`
  - Web: `downloadPcapng()` (UI “Download capture”)
  - Clears the in-memory buffer after exporting, but does **not** reset drop counters.
- **Snapshot export**:
  - Rust: `NetTracer.export_pcapng()`
  - Web: `exportPcapng()` (UI “Download snapshot”, when supported)
  - Exports without clearing the in-memory buffer.
- **Clear**:
  - Rust: `NetTracer.clear()`
  - Web: `clear()` (UI “Clear capture”, when supported)
  - Drops buffered records *and* resets drop counters.

Operational note: for long debugging sessions, prefer periodically taking a draining export (or clearing)
to keep memory bounded and to make drop counters meaningful over a known time window.

Performance note: enabling tracing copies each captured frame into an in-memory buffer (so it has
CPU + memory overhead). Keep tracing disabled unless actively debugging.

Server-side alternative: the production L2 proxy can optionally write per-session `.pcapng` files via
`AERO_L2_CAPTURE_DIR` (see **L2 proxy observability** below). Capturing on both ends can help debug
tunnel framing vs. proxy-side stack issues; note that proxy-side capture is also bounded and may be
truncated depending on its capture limits.

#### Privacy / Security Warning

Captures may include sensitive data such as credentials, cookies, private browsing traffic, or internal
network metadata. Tracing must default to off and the UI should warn users before enabling or
exporting captures.

Rust-side tracing supports optional redaction via `NetTraceConfig.redactor` (a `NetTraceRedactor`),
which can drop records (`None`) or truncate payloads to reduce sensitive capture.

---

### Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| TCP Latency | < 100ms | Additional over native |
| TCP Throughput | ≥ 10 Mbps | Typical web traffic |
| UDP Latency | < 50ms | For real-time apps |
| Connection Setup | < 500ms | Including WebSocket |

---

### Next Steps

- See [Input Devices](usb-and-input.md) for keyboard/mouse
- See [Browser APIs](web-host.md) for WebSocket/WebRTC details
- See [Task Breakdown](../history/sprint-era-record.md) for network tasks

---

### Gateway observability (health, readiness, metrics)

The gateway (`services/gateway`) exposes operational endpoints intended for monitoring and automation:

- `GET /healthz` – liveness
- `GET /readyz` – readiness (returns `503` while shutting down)
- `GET /version` – build/version info (for deploy/debug)
- `GET /metrics` – Prometheus metrics (HTTP request totals/latency; DNS metrics once DoH is enabled)

**Operational guidance:** expose these endpoints only on trusted networks (or behind auth). They are not intended as a public API surface.

For traffic-level debugging, prefer:

- client-side PCAP/trace exports (see **Network Tracing (PCAP/PCAPNG Export)** above), and/or
- infrastructure-level capture (reverse proxy logs, host packet capture in controlled environments).

### L2 proxy observability (health, readiness, metrics, capture)

The production L2 proxy (`crates/aero-l2-proxy`) also exposes basic operational endpoints:

- `GET /healthz` – liveness
- `GET /readyz` – readiness
- `GET /version` – build/version info
- `GET /metrics` – Prometheus metrics (tunnel/session counters + stack/proxy activity)

It also supports optional per-session traffic capture for debugging:

- `AERO_L2_CAPTURE_DIR=/path/to/dir` – when set, writes one `.pcapng` file per tunnel session containing:
  - guest→proxy Ethernet frames (inbound)
  - proxy→guest Ethernet frames (outbound)
- `AERO_L2_CAPTURE_MAX_BYTES=67108864` – maximum **capture file size** written per session file (default: **64 MiB**; `0` disables the cap). Note: this includes PCAPNG container overhead, not just Ethernet payload bytes.
- `AERO_L2_CAPTURE_FLUSH_INTERVAL_MS=1000` – flush interval for capture writers (default: **1000ms**; `0` disables periodic flushing; capture is flushed on close).
- `AERO_L2_PING_INTERVAL_MS=1000` – when set, the proxy sends protocol-level PINGs; RTT is recorded as `l2_ping_rtt_ms` histogram in `/metrics`.

Capture is **best-effort**:

- It may be **truncated** when the max-bytes cap is reached, or if file I/O fails.
- Verify completeness via capture metrics in `/metrics`:
  - `l2_capture_frames_total` / `l2_capture_bytes_total` (written)
  - `l2_capture_frames_dropped_total` (frames skipped due to the max-bytes cap; non-zero implies an incomplete capture)
  - `l2_capture_errors_total` (capture I/O errors; non-zero implies the capture may be incomplete)

## Networking Architecture RFC: slirp-in-browser vs L3/L2 tunneling

> **Final decision:** [the L2 tunnel networking decision: Networking via L2 tunnel (Option C) to an unprivileged proxy](../decisions/0013-networking-l2-tunnel.md).  
> This RFC is retained for background, tradeoff analysis, and prototype references.

### Context / goal

Aero needs guest networking (Windows 7 TCP/IP stack → emulated NIC) to reach:

- DNS (name resolution)
- TCP (web browsing, Windows Update, etc.)
- UDP (DNS, NTP, many games/VoIP apps; eventually required)

Browser constraints:

- No raw sockets, no direct TCP/UDP to arbitrary hosts.
- Only browser transports (WebSocket, WebRTC DataChannel) and HTTP(S) are available.
- Therefore **some form of proxy** is required for real networking.

This RFC resolves where the “host-side network stack” lives:

- **A)** in the browser (WASM slirp/NAT) with a “dumb” relay proxy
- **B)** tunnel **IP packets** (L3) to a proxy that does DHCP/DNS/NAT
- **C)** tunnel **Ethernet frames** (L2) to a proxy that does bridging or a user-space stack

---

### Option A — In-browser slirp/NAT (current doc direction)

#### Summary

Implement a user-mode network stack in the browser (WASM):

- ARP + DHCP server/client behavior
- IPv4 routing
- UDP and **TCP termination** (slirp-style)
- NAT + port mapping
- DNS forwarding (DoH or proxy-assisted)

Then translate guest sockets to browser-available transports:

- TCP → one WebSocket per connection (or multiplexed)
- UDP → WebRTC DataChannel (or WebSocket with framing)

#### Pros

- **No per-packet tunneling over the WAN**: can translate guest TCP segments into a stream early,
  reducing ACK chatter and head-of-line effects compared to packet tunneling.
- **Unprivileged server**: proxy can be “just” a TCP/UDP relay (no TUN/TAP).
- **Potentially better guest-perceived connect latency**: slirp can SYN-ACK locally before the
  upstream connect finishes (at the cost of buffering/edge cases).

#### Cons

- **Very large implementation surface in the browser**:
  implementing TCP correctly (retransmits, window scaling, SACK, PMTU discovery, corner cases)
  is a multi-month project and hard to debug.
- **CPU budget conflict**: Aero’s critical constraint is browser CPU time (emulation/JIT/WebGPU).
  A TCP/IP stack in the same process competes directly with emulation performance.
- **Feature completeness pressure**: Windows will exercise “weird” network behaviors
  (ICMP, fragmentation, DHCP renew, DNS retries, etc.).
- **Harder security story client-side**: filtering/quotas can be enforced in the proxy, but the
  browser still has to parse and synthesize complex protocol state robustly.

#### Security implications

- Proxy still effectively becomes an **open egress** endpoint unless targets are restricted.
- The browser stack can be used for exfiltration (expected), but also increases attack surface
  for memory/CPU exhaustion in the emulator runtime.

---

### Option B — L3 tunnel (IP packets) to proxy; proxy does NAT + DHCP/DNS

#### Summary

Browser forwards **IP packets** to a proxy server; proxy injects packets into a network stack
and returns IP packets back.

Typical implementation variants:

1) **Kernel stack via TUN** (privileged): create a TUN device per VM session and use iptables/NAT.
2) **User-space stack** (unprivileged): run a TCP/IP stack (gVisor netstack, smoltcp-based, etc.)
   and perform NAT in process.

The browser still needs to deal with the fact the guest NIC is L2:

- Either emulate a point-to-point link (custom guest driver), **or**
- Handle ARP/DHCP at the browser boundary and tunnel only IPv4 payloads.

#### Pros

- **No TCP implementation in the browser**: the guest’s stack speaks TCP; the proxy handles the
  “host side”.
- Proxy can centralize **policy enforcement** (egress allowlist/denylist, rate limits, logging).
- Potentially smaller bandwidth than L2 tunneling (no Ethernet header/broadcast frames).

#### Cons

- If using kernel via TUN: **requires CAP_NET_ADMIN/root-ish privileges** and host networking
  configuration (routing, NAT) that many PaaS/serverless environments disallow.
- If using user-space stack: still non-trivial implementation (but server-side is easier to iterate).
- Browser boundary still has **L2 impedance mismatch** unless a custom guest driver is introduced.

#### Security implications

- Same core SSRF/open-proxy risk as Option A: the proxy can be induced to connect to internal IPs.
  Mitigations belong on the proxy side (deny RFC1918/ULA/link-local by default, per-tenant policy).

---

### Option C — L2 tunnel (Ethernet frames) to proxy; proxy provides bridge/TAP or user-space stack

#### Summary

Browser forwards **raw Ethernet frames** from the emulated NIC to the proxy, and receives frames
back from the proxy.

**Wire protocol:** see [`../specs/l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md) (versioned framing +
PING/PONG + size limits).

Proxy implementation variants:

1) **Kernel bridge via TAP** (privileged): create TAP per VM session, bridge/NAT in the host.
2) **User-space “slirp on the server”** (unprivileged): run a user-mode stack that speaks Ethernet
   and uses normal host sockets for outbound (libslirp-style).

#### Pros

- **Simplest browser boundary**: the browser does not need to understand ARP/DHCP/IPv6.
  It is a pure frame forwarder: `virtio-net/e1000 ↔ tunnel ↔ proxy`.
- **Most protocol-complete** at the boundary: any L2/L3 protocol Windows emits can be carried
  without client-side special cases.
- **Proxy-side policy** remains centralized (egress controls, quotas, auditing).
- With a user-space stack, the proxy can run **unprivileged** (no TUN/TAP), which is important
  for hosted multi-tenant deployments.

#### Cons

- **More bytes over the tunnel** than socket-level relaying:
  TCP ACKs, retransmits, and broadcast traffic traverse the WAN.
- **Transport choice matters**:
  - WebSocket is reliable but suffers head-of-line blocking (HOL).
  - WebRTC DataChannel can be tuned (ordering + reliability). Unordered delivery can reduce HOL, but
    Aero’s current L2 tunnel requires an **ordered** reliable DataChannel for correctness; see
    [`l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md).
  - If using TAP/bridge: requires CAP_NET_ADMIN and increases risk of exposing a VM to the proxy’s
    L2 environment if misconfigured.

#### Security implications

- A TAP/bridge design can accidentally expose the VM to a real L2 segment (ARP spoofing, scanning).
  Strong isolation is required; for hosted SaaS this is a major operational footgun.
- User-space stack avoids bridging to the host LAN entirely; VM sees only a synthetic LAN.

---

### Performance / latency expectations (qualitative)

| Axis | A) Browser slirp | B) L3 tunnel | C) L2 tunnel |
|------|------------------|--------------|--------------|
| Browser CPU | High (TCP/IP stack) | Low–Medium | Low |
| Proxy CPU | Low | Medium | Medium |
| Bandwidth overhead | Low–Medium | Medium | Highest |
| Latency sensitivity | Lower (can ACK locally) | Higher (packet RTT adds) | Higher (packet RTT adds) |
| Correctness surface | Large in browser | Large on proxy | Large on proxy |
| Operational privilege | None | Often needs TUN | Often needs TAP |

Important note: with WebRTC (unordered / partial reliability options), the practical latency
difference between packet tunneling and socket relaying is often dominated by the proxy’s geographic
distance and congestion, not protocol details.

---

### Recommendation

**Recommend Option C: L2 tunnel (Ethernet frames) to a proxy that runs an unprivileged user-space
network stack (server-side slirp/NAT).**

For the concrete wire protocol and deployment notes, see:

- [`l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md)
- [`l2-tunnel-runbook.md`](networking.md)

#### Rationale

1) **Browser simplicity + emulator performance**: Aero’s performance bottleneck is the client.
   Keeping the browser as a frame forwarder avoids dedicating substantial CPU to TCP/IP.
2) **Protocol completeness at the boundary**: Windows networking is complicated. L2 tunneling
   avoids a long tail of “why does Windows send this?” client-side bugs.
3) **Avoid privileged networking in the proxy**: requiring TUN/TAP/CAP_NET_ADMIN limits deployment
   options and increases operational risk. A user-space stack can run in standard containers.
4) **Centralized security controls**: the proxy can enforce egress policy (deny RFC1918 by default,
   rate limits, per-session caps) regardless of what Windows attempts.

#### Implications for follow-on work (NT-STACK / NT-WS-PROXY / NT-WRTC-PROXY)

- The browser networking implementation becomes a **transport + framing layer**:
  it forwards frames between the emulated NIC and the tunnel (WebSocket initially; WebRTC later).
- The proxy implements:
  - DHCP + DNS services for the VM subnet
  - NAT (TCP/UDP) to the public internet
  - Policy controls (allow/deny, quotas) and observability

---

### Implementation plan (repo components)

The Option C implementation is split into a small set of concrete components:

- `apps/web/src/net/l2Tunnel.ts`
  - Browser-side tunnel client.
  - Owns the WebSocket (default) or WebRTC DataChannel (optional) and forwards raw Ethernet frames
    between the emulator and the proxy.
- `crates/aero-net-backend`
  - Emulator-side L2 tunnel backends that the NIC device model calls into, instead of running a full
    TCP/IP stack in WASM.
  - Includes:
    - queue-backed `L2TunnelBackend` (in-memory FIFO queues; useful for native test harnesses and
      non-browser hosts), and
    - ring-buffer-backed `L2TunnelRingBackend` for the browser runtime (`NET_TX`/`NET_RX` AIPC queues
      in `ioIpcSab`).
- `crates/aero-l2-proxy`
  - Unprivileged proxy service implementing a user-space Ethernet/IP stack + NAT + policy.
  - Terminates the L2 tunnel and returns frames (ARP/DHCP/DNS/etc.) back to the browser.
- `proxy/webrtc-udp-relay`
  - Optional WebRTC transport for the L2 tunnel (DataChannel carrying the L2 tunnel framing).
  - Also remains useful for standalone UDP relay use-cases during migration.

---

### Prototype in this repo

This RFC is accompanied by a minimal prototype that demonstrates the Option C shape:

**Security note:** this prototype is for experimentation only and is not hardened for production use.
For a maintained, policy-driven L2 tunnel proxy implementation, use `crates/aero-l2-proxy`
(see [`networking.md`](networking.md)) and treat it as a security-critical egress
surface (enforce strict policy and quotas).
For the legacy socket-level relays (Phase 0 `/tcp`, DoH, UDP datagrams), use:

- `services/gateway` for TCP + DNS (`/tcp`, `/tcp-mux`, `/dns-query`, `/dns-json`; see `services/gateway/openapi.yaml`)
- `proxy/webrtc-udp-relay` for UDP datagrams (`/webrtc/*`, `/udp`; see `proxy/webrtc-udp-relay/PROTOCOL.md`)
  - DoS hardening: the relay configures pion/SCTP message-size caps to bound receive-side buffering/allocation before `DataChannel.OnMessage` runs (`WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES` + `WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES`) and closes half-open sessions after `WEBRTC_SESSION_CONNECT_TIMEOUT`.
- `services/net-proxy/` can be used as a local dev relay (`/tcp`, `/tcp-mux`, `/udp`, plus DoH `/dns-query` + `/dns-json`).
  - Note: the DoH endpoints are `fetch()`-based; for browser clients you generally want them to be same-origin with your
    frontend (e.g. proxy via Vite), or enable the explicit CORS allowlist (`AERO_PROXY_DOH_CORS_ALLOW_ORIGINS`).
- `tools/aero-gateway-rs` is an older Rust/Axum gateway prototype kept only for
  **legacy/diagnostic** purposes (historical `/tcp?target=<host>:<port>`). It is not
  production-hardened; the canonical gateway is `services/gateway`.

**Protocol note:** the prototype now uses the versioned L2 tunnel framing (including basic
PING/PONG handling) over WebSocket. Production implementations should use the maintained codec and
enforce all limits described in
[`../specs/l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md).

- Client (“browser side”) sends:
  - ARP request (to discover gateway MAC)
  - DNS query (UDP/53)
  - TCP SYN + data to an echo server
- Proxy responds with ARP/DNS/TCP frames and forwards TCP payload to a real TCP socket.

See:

- `tests/helpers/proxy-server.js`
- `tests/helpers/client.js`
- `tests/networking-architecture-rfc.test.js`

---

### Current implementation status

What exists today (in repo):

- **Option C (L2 tunnel):**
  - Wire protocol: `../specs/l2-tunnel-protocol.md` (`aero-l2-tunnel-v1`)
  - Browser client: `apps/web/src/net/l2Tunnel.ts`
  - Proxy: `crates/aero-l2-proxy` (`GET /l2` WebSocket (legacy alias: `/eth`), user-space stack + NAT)
  - Optional WebRTC transport bridge: `proxy/webrtc-udp-relay` (`l2` DataChannel ↔ backend WS `/l2`, per `PROTOCOL.md`)
- **Phase 0 / migration (socket-level relays):**
  - `services/gateway` (`POST /session`, `/tcp`, `/tcp-mux`, `/dns-query`, `/dns-json`, `/udp-relay/token`)
  - `proxy/webrtc-udp-relay` UDP relay (`udp` DataChannel framing v1/v2 + WebSocket `/udp` fallback)

What Option C still requires to fully replace Phase 0:

- Making the L2 tunnel the default path in production builds (with clear fallbacks for debugging).
- Continued hardening of the proxy egress policy, observability, and resource accounting under real workloads.

## L2 tunnel runbook (Option C)

This document is a practical guide for running Aero’s **Option C** networking path:
**tunnel raw Ethernet frames (L2)** between the browser and a proxy that runs the user-space NAT
stack, using the versioned framing described in [`l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md).

**Final decision:** [the L2 tunnel networking decision: Networking via L2 tunnel (Option C) to an unprivileged proxy](../decisions/0013-networking-l2-tunnel.md).
For background/tradeoffs, see [`networking-architecture-rfc.md`](networking.md).
For the wire protocol/framing, see [`l2-tunnel-protocol.md`](../specs/l2-tunnel-protocol.md).

### Local development

#### Start the L2 proxy (WebSocket data plane)

The recommended dev setup uses a **single WebSocket per VM session** carrying the L2 tunnel framing
as binary messages.

Start the proxy service.

Current implementation in this repo (production target: L2 tunnel termination + user-space NAT stack + egress policy):

```bash
cargo run --locked -p aero-l2-proxy

# Optional: override listen address (default: 0.0.0.0:8090)
# AERO_L2_PROXY_LISTEN_ADDR=127.0.0.1:8090 cargo run --locked -p aero-l2-proxy

# Security knobs (Rust `crates/aero-l2-proxy`):
# - Origin is enforced by default; configure an allowlist for your dev origin:
#   AERO_L2_ALLOWED_ORIGINS=http://localhost:5173 cargo run --locked -p aero-l2-proxy
#   # (or use the shared name supported by the gateway + WebRTC relay)
#   ALLOWED_ORIGINS=http://localhost:5173 cargo run --locked -p aero-l2-proxy
#   # Optionally append additional origins (comma-prefixed convention):
#   ALLOWED_ORIGINS=https://localhost AERO_L2_ALLOWED_ORIGINS_EXTRA=",http://localhost:5173" cargo run --locked -p aero-l2-proxy
# - Trusted local dev escape hatch (disables Origin enforcement):
#   AERO_L2_OPEN=1 cargo run --locked -p aero-l2-proxy
# - Authentication (recommended for any internet-exposed deployment):
#   - Session cookie (same-origin browser sessions; requires `aero_session` cookie from `POST /session`):
#     AERO_L2_AUTH_MODE=session AERO_L2_SESSION_SECRET=sekrit cargo run --locked -p aero-l2-proxy
#     (The proxy must share the gateway session signing secret: `SESSION_SECRET` / `AERO_GATEWAY_SESSION_SECRET`.)
#     (Legacy alias: `AERO_L2_AUTH_MODE=cookie`.)
#   - Token (simple cross-origin / server-to-server; dev-only if long-lived):
#     AERO_L2_AUTH_MODE=token AERO_L2_API_KEY=sekrit cargo run --locked -p aero-l2-proxy
#     (Deprecated compatibility: legacy `AERO_L2_TOKEN=sekrit` is treated as a token when
#     `AERO_L2_AUTH_MODE` is unset, and is also accepted as a fallback value for `AERO_L2_API_KEY`.)
#     (Legacy alias: `AERO_L2_AUTH_MODE=api_key`.)
#   - JWT (recommended for cross-origin / short-lived tokens):
#     AERO_L2_AUTH_MODE=jwt AERO_L2_JWT_SECRET=sekrit cargo run --locked -p aero-l2-proxy
#     # Optional claim enforcement:
#     # AERO_L2_JWT_AUDIENCE=aero AERO_L2_JWT_ISSUER=aero-gateway
#   - Mixed/hybrid modes:
#     - Cookie + JWT:
#       AERO_L2_AUTH_MODE=cookie_or_jwt AERO_L2_SESSION_SECRET=sekrit AERO_L2_JWT_SECRET=sekrit cargo run --locked -p aero-l2-proxy
#     - Session cookie + token (accept either):
#       AERO_L2_AUTH_MODE=session_or_token AERO_L2_SESSION_SECRET=sekrit AERO_L2_API_KEY=sekrit cargo run --locked -p aero-l2-proxy
#       # (If AERO_L2_API_KEY is omitted, the proxy still accepts session-cookie auth; the token path is simply disabled.)
#     - Session cookie + token (require both):
#       AERO_L2_AUTH_MODE=session_and_token AERO_L2_SESSION_SECRET=sekrit AERO_L2_API_KEY=sekrit cargo run --locked -p aero-l2-proxy
#   - Credential delivery:
#     - query params: `?token=...` (or `?apiKey=...` for compatibility)
#     - subprotocol token: additional `Sec-WebSocket-Protocol` entry `aero-l2-token.<credential>`
#       (offered alongside `aero-l2-tunnel-v1`)
#     - JWTs can also be provided via `Authorization: Bearer <token>` when using a non-browser client.
#
# - Quotas:
#   - AERO_L2_MAX_CONNECTIONS=64                 # process-wide concurrent tunnel cap (`0` disables)
#   - AERO_L2_MAX_CONNECTIONS_PER_SESSION=0      # per-session concurrent tunnel cap (`0` disables; legacy alias: AERO_L2_MAX_TUNNELS_PER_SESSION)
#
# - Payload size limits (defense in depth):
#   - AERO_L2_MAX_FRAME_PAYLOAD=2048             # max FRAME payload bytes (default/recommended: 2048; legacy alias: AERO_L2_MAX_FRAME_SIZE)
#   - AERO_L2_MAX_CONTROL_PAYLOAD=256            # max control payload bytes (default/recommended: 256)
#   - Values must be positive integers; `0`/blank are treated as unset (defaults apply).
#   Note: `services/gateway` surfaces these values via `POST /session` when the env vars are set,
#   so set them on the gateway as well if you override them on the proxy.
#
# Observability knobs:
# - Optional: per-session PCAPNG capture (writes one file per tunnel session):
#   AERO_L2_CAPTURE_DIR=/tmp/aero-l2-captures cargo run --locked -p aero-l2-proxy
#   # Optional capture limits/tuning:
#   AERO_L2_CAPTURE_MAX_BYTES=67108864             # default: 64 MiB per session capture file (includes PCAPNG overhead; `0` disables the cap)
#   AERO_L2_CAPTURE_FLUSH_INTERVAL_MS=1000         # default: 1000ms (`0` disables periodic flushing; capture is flushed on close)
# - Optional: have the proxy send protocol-level PINGs (RTT is recorded in Prometheus metrics):
#   AERO_L2_PING_INTERVAL_MS=1000 cargo run --locked -p aero-l2-proxy
```

Expected behavior:

- The proxy listens on `AERO_L2_PROXY_LISTEN_ADDR` (default: `0.0.0.0:8090`).
- Operational endpoints:
  - `GET /healthz` – liveness
  - `GET /readyz` – readiness
  - `GET /version` – build/version info
  - `GET /metrics` – Prometheus metrics
- WebSocket tunnel endpoint:
  - `GET /l2` (legacy alias: `/eth`) – L2 tunnel (subprotocol `aero-l2-tunnel-v1`)
- The proxy is configured with a strict egress policy in production; local dev may enable “open” mode.

##### Browser: establish the L2 tunnel over WebSocket

In the browser, create a WebSocket L2 tunnel client and connect it to the proxy:

```ts
import { WebSocketL2TunnelClient } from "./net";

// `gatewayBaseUrl` can be:
// - `ws://...` / `wss://...` (explicit WebSocket URL), or
// - `http://...` / `https://...` (auto-converted to ws(s) and `/l2` appended), or
// - a same-origin path like `/l2` (legacy alias: `/eth`) when running the full web app.
const l2 = new WebSocketL2TunnelClient("http://127.0.0.1:8090", (ev) => {
  if (ev.type === "frame") nicRx(ev.frame);
  if (ev.type === "error") console.error(ev.error);
});

l2.connect();
// `sendFrame()` returns a boolean indicating whether the frame was accepted into
// the client's outbound queue; most callers can ignore it.
nicTx = (frame) => {
  l2.sendFrame(frame);
};
```

If the proxy requires token-based auth (`AERO_L2_AUTH_MODE=token|jwt`, or the legacy
`AERO_L2_TOKEN` alias), pass a credential and choose how it is transported:

```ts
const l2 = new WebSocketL2TunnelClient("ws://127.0.0.1:8090", sink, {
  token: "sekrit",
  // Default is "query" (adds ?token=...); "subprotocol" uses an additional
  // Sec-WebSocket-Protocol entry `aero-l2-token.<token>` alongside `aero-l2-tunnel-v1`.
  // Prefer "subprotocol" when possible to avoid putting secrets in URLs/logs; use "query" when
  // the credential isn't a valid HTTP token (RFC 7230 `tchar`) or the client cannot set subprotocols.
  tokenTransport: "subprotocol",
});
```

Note: non-browser clients can alternatively provide JWT credentials via an `Authorization: Bearer <token>`
header (supported by `crates/aero-l2-proxy` in `AERO_L2_AUTH_MODE=jwt|cookie_or_jwt`).

##### Browser-side observability (worker runtime)

When running the full worker-based emulator runtime, the network worker emits low-rate `log` events
on the runtime event ring to help debug L2 tunnel bring-up/backpressure:

- Look for `[net] l2: ...` logs in the dev console (e.g. `l2: open tx=... rx=... drop+{...}`).
- Connection transitions (`l2: connecting/open/closed/error`) are logged immediately.
- When drop deltas are non-zero, the periodic stats log is emitted at `WARN` so it also appears in
  the coordinator's nonfatal stream.
- Tunnel transport errors are emitted at `ERROR` (`l2: error: ...`).

#### (Optional) Start the WebRTC relay (DataChannel transport)

WebRTC is optional. Use it when you want a UDP-based tunnel transport for experimentation and
evaluation under loss.

If you carry the **L2 tunnel** over WebRTC, the DataChannel must be configured as:

- **reliable** (no frame loss / no partial reliability)
- **ordered** (`ordered = true`)
- leave `maxRetransmits` / `maxPacketLifeTime` unset (default reliable)

See the L2 tunnel networking decision for the rationale.

The existing relay implementation lives at `proxy/webrtc-udp-relay/`:

```bash
cd proxy/webrtc-udp-relay

 # Bridge WebRTC DataChannel "l2" to the L2 proxy WebSocket endpoint.
 # (The relay will forward the client's Origin + AUTH_MODE credential by default.)
 export L2_BACKEND_WS_URL=ws://127.0.0.1:8090/l2
  
 # Optional knobs:
 # If the backend uses session-cookie auth (`AERO_L2_AUTH_MODE=session` on `crates/aero-l2-proxy`):
 # export L2_BACKEND_FORWARD_AERO_SESSION=1   # forwards Cookie: aero_session=... captured from signaling
 # export L2_BACKEND_AUTH_FORWARD_MODE=query        # default
 # export L2_BACKEND_AUTH_FORWARD_MODE=subprotocol  # offer Sec-WebSocket-Protocol entry aero-l2-token.<credential> alongside aero-l2-tunnel-v1 (credential must be a valid HTTP token / RFC 7230 tchar)
 # If your backend requires a static token (e.g. `AERO_L2_AUTH_MODE=token|jwt` on `crates/aero-l2-proxy`):
  # export L2_BACKEND_TOKEN=sekrit                   # offer Sec-WebSocket-Protocol entry aero-l2-token.<token> alongside aero-l2-tunnel-v1 (token must be a valid HTTP token / RFC 7230 tchar)
  # export L2_BACKEND_AUTH_FORWARD_MODE=none         # don't also forward client creds in ?token=/?apiKey=
  # export L2_BACKEND_ORIGIN_OVERRIDE=https://example.com
  # export L2_BACKEND_ORIGIN=https://example.com # alias for L2_BACKEND_ORIGIN_OVERRIDE
  #
  # WebRTC relay DoS hardening (optional tuning; defaults are derived and safe):
  # - WEBRTC_DATACHANNEL_MAX_MESSAGE_BYTES (SDP `a=max-message-size` hint; 0 = auto)
  # - WEBRTC_SCTP_MAX_RECEIVE_BUFFER_BYTES (hard receive-side cap; 0 = auto)
  # - WEBRTC_SESSION_CONNECT_TIMEOUT (close half-open sessions; default 30s)
   go run ./cmd/aero-webrtc-udp-relay
```

See [`proxy/webrtc-udp-relay/README.md`](../decisions/README.md) for TURN/docker-compose
notes and security controls.

##### Browser: establish the L2 tunnel over WebRTC

In the browser, use the helper in `apps/web/src/net/l2RelaySignalingClient.ts` to negotiate a
`RTCPeerConnection` against the relay and obtain a **fully reliable and ordered** `RTCDataChannel` labeled `l2`:

 ```ts
 import { connectL2Relay } from "./net";
 
 const { l2, close } = await connectL2Relay({
  // `baseUrl` can be http(s):// or ws(s):// depending on how the relay is
  // exposed (some deployments share a single wss:// origin behind a reverse
  // proxy). The browser client will normalize schemes per endpoint transport.
  baseUrl: "https://relay.example.com", // (or "wss://relay.example.com")
  authToken: "…", // optional
  mode: "ws-trickle", // default
  sink: (ev) => {
    if (ev.type === "frame") {
      nicRx(ev.frame);
    }
  },
});

nicTx = (frame) => {
  l2.sendFrame(frame);
};
// Later: close();
```

#### Run the RFC-style probe (ARP + DHCP + DNS + TCP echo)

The fastest sanity check for an L2 tunnel is to run the RFC prototype probe, which exercises the
expected “minimum viable LAN” behaviors:

- **ARP** (discover gateway MAC)
- **DHCP** (obtain guest IP + gateway + DNS)
- **DNS** (UDP/53 query/response)
- **TCP echo** (SYN/SYN-ACK/ACK + data roundtrip)

Automated probe (ARP + DNS + TCP echo) lives in this repo today:

```bash
node --test tests/networking-architecture-rfc.test.js
```

This test spins up a minimal WebSocket frame-forwarding proxy and a local TCP echo server, then
performs the probe over Ethernet frames wrapped in the `aero-l2-tunnel-v1` framing.

DHCP verification (until the automated probe covers it):

- Boot a guest and confirm it receives a lease (Windows: `ipconfig /all`).
- Force renewal (Windows: `ipconfig /renew`) and capture traffic with the built-in PCAP tracing
  hooks described in [`07-networking.md`](networking.md#network-tracing-pcappcapng-export)
  (browser UI panel “Network trace (PCAPNG)” / `window.aero.netTrace.downloadPcapng()`).

### Secure deployment (recommended)

Treat the L2 proxy as a **high-risk network egress surface**. A secure deployment needs:
**Origin enforcement + authentication + egress policy**.

#### Origin allowlist (required)

By default, `crates/aero-l2-proxy` requires an `Origin` header on the WebSocket upgrade request and
validates it against an allowlist:

- `AERO_L2_ALLOWED_ORIGINS`: comma-separated list of allowed origins.
  - If unset/empty, falls back to `ALLOWED_ORIGINS` (shared with the gateway + WebRTC relay).
  - `AERO_L2_ALLOWED_ORIGINS_EXTRA` (optional) is appended (comma-prefixed convention used by `deploy/docker-compose.yml`).
  - Origins are normalized before comparison (see `../specs/l2-tunnel-protocol.md` for rules/examples).
  - Example: `https://app.example.com,https://staging.example.com`
  - `*` allows any **valid** Origin value (still requires the header to be present).

Dev escape hatch:

- `AERO_L2_OPEN=1` disables Origin enforcement (trusted local development only).

#### Authentication (required)

Origin enforcement is not sufficient to protect an internet-exposed L2 endpoint: non-browser
clients can omit or forge `Origin`.

`crates/aero-l2-proxy` supports multiple auth modes via `AERO_L2_AUTH_MODE`:

##### a) Same-origin browser clients (recommended): session-cookie auth

- Set `AERO_L2_AUTH_MODE=session` (legacy alias: `cookie`).
- Ensure the proxy shares the gateway session signing secret:
  - `SESSION_SECRET` (gateway), and
  - `AERO_L2_SESSION_SECRET` (or `SESSION_SECRET` / `AERO_GATEWAY_SESSION_SECRET`) on the L2 proxy must match.

Single-origin flow:

1) Browser calls `POST /session` on the gateway (same origin) to receive the `aero_session` cookie
2) Browser opens `wss://<origin>/l2` (subprotocol `aero-l2-tunnel-v1`) and relies on the cookie

##### b) Cross-origin / non-browser clients: token or JWT

- **JWT** (recommended):
  - Configure: `AERO_L2_AUTH_MODE=jwt` and `AERO_L2_JWT_SECRET=...`
  - Optional claim validation: `AERO_L2_JWT_AUDIENCE` and/or `AERO_L2_JWT_ISSUER`
- **Token** (simpler, but avoid long-lived keys for public deployments):
  - Configure: `AERO_L2_AUTH_MODE=token` and `AERO_L2_API_KEY=...` (legacy alias: `api_key`)
    - Legacy alias: `AERO_L2_TOKEN=...` (used when `AERO_L2_AUTH_MODE` is unset; also accepted as a
      fallback value for `AERO_L2_API_KEY`).
- Optional mixed modes:
  - `AERO_L2_AUTH_MODE=cookie_or_jwt` accepts either a session cookie or a JWT.
  - `AERO_L2_AUTH_MODE=session_or_token` accepts either a session cookie or a token.

Credentials offered via `Sec-WebSocket-Protocol` must be valid WebSocket subprotocol tokens (HTTP token / RFC 7230 `tchar`).
Prefer subprotocol delivery when possible to avoid putting secrets in URLs/logs; use query-string delivery when the credential
cannot be expressed as a subprotocol token.

- Query string: `wss://proxy.example.com/l2?token=<value>` (or `?apiKey=<value>` for compatibility)
- Header: `Authorization: Bearer <token>` (JWT only)
- WebSocket subprotocol: offer an additional `Sec-WebSocket-Protocol` entry `aero-l2-token.<value>`
  alongside `aero-l2-tunnel-v1`.

Missing/incorrect credentials reject the upgrade with **HTTP 401** (no WebSocket).

#### WebRTC L2 bridging (relay forwards auth + Origin)

When carrying the L2 tunnel over WebRTC, the browser connects to `proxy/webrtc-udp-relay` and the
relay opens a backend WebSocket to `aero-l2-proxy` and bridges:

- **auth** (cookie and/or token, depending on relay config), and
- **Origin** (forwarded so the backend can enforce the same allowlist).

Enable the backend wiring with these environment variables (the compose stack that used to carry them in `deploy/.env` was purged; set them however you run the relay):

```bash
L2_BACKEND_WS_URL=ws://aero-l2-proxy:8090/l2
L2_BACKEND_AUTH_FORWARD_MODE=query
L2_BACKEND_FORWARD_ORIGIN=1
L2_BACKEND_FORWARD_AERO_SESSION=1  # recommended when `aero-l2-proxy` uses AERO_L2_AUTH_MODE=session
```

Then ensure auth is compatible end-to-end:

- If using session-cookie auth (`AERO_L2_AUTH_MODE=session`), ensure `aero-l2-proxy` shares the gateway `SESSION_SECRET`
  and the relay has `L2_BACKEND_FORWARD_AERO_SESSION=1` enabled.
- If using token auth, configure `aero-l2-proxy` with `AERO_L2_AUTH_MODE=token` and `AERO_L2_API_KEY=...`
  (or the legacy `AERO_L2_TOKEN=...` alias).
- Configure the relay auth mode (`AUTH_MODE=jwt` or `AUTH_MODE=api_key`) so the browser presents a
  credential that the relay can forward to the backend as `?token=...` (or via `aero-l2-token.*` when
  `L2_BACKEND_AUTH_FORWARD_MODE=subprotocol` is used).

### Production checklist

Treat the L2 proxy as a **high-risk network egress surface**. A secure deployment requires policy
and hardening at the proxy boundary.

Minimum checklist:

- **Origin allowlist**
  - Enforce `Origin` on WebSocket upgrades; enforce strict CORS on any HTTP endpoints.
  - Consider also validating `Host` / `X-Forwarded-Host` when behind a reverse proxy.
- **Auth + session binding**
  - Browser clients: require a cookie-backed gateway session (`aero_session`).
  - Non-browser/internal clients: require a short-lived token (prefer the WebSocket subprotocol form).
  - Bind tunnel sessions to an authenticated user and enforce per-user quotas.
- **Blocked destination ranges**
  - Deny loopback, link-local, RFC1918, CGNAT, multicast, and other special-use ranges by default.
  - Re-check after DNS resolution to prevent DNS rebinding (hostnames that resolve to internal IPs).
- **Port allowlist**
  - Default-deny outbound ports; allow only what you intend to support (typically 80/443 plus a
    small set of well-known ports if needed).
- **Quotas / rate limits**
  - Max concurrent sessions per user/IP.
  - Max concurrent TCP/UDP flows per session.
  - Byte/packet rate limits and burst limits (protects CPU and upstream bandwidth).

Operational recommendations:

- Log enough metadata to audit abuse (destination IP/port, byte counts, auth principal), but avoid
  logging raw payload bytes by default.
- Provide `/healthz`, `/readyz`, and basic metrics endpoints for monitoring.
- Consider explicit “open dev mode” toggles so production never accidentally runs with permissive
  settings.
