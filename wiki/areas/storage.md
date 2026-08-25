# Storage

> Windows 7 needs tens of gigabytes of disk with random access from inside a
> browser tab. This page covers the whole chain: host persistence, byte
> backends, disk image formats and overlays, and the emulated controllers.
> Delivery of multi-gigabyte images over HTTP, and the hosted lifecycle around
> it, are specified in
> [../specs/disk-image-delivery-and-lifecycle.md](../specs/disk-image-delivery-and-lifecycle.md).

---

## What this area covers

Windows 7 needs 15–40 GB disks and random access to them from inside a
browser. The storage area therefore spans four layers:

1. **Host persistence** — OPFS files, IndexedDB, in-memory, network.
2. **Byte backends** — synchronous random-access storage traits.
3. **Disk image formats / wrappers** — raw, AeroSparse, QCOW2, VHD, COW
   overlays, block cache.
4. **Device/controller models** — AHCI, IDE/ATAPI, NVMe, virtio-blk, plus the
   BIOS INT 13h view of the same disks.

Plus the delivery side: streaming multi-GB images over HTTP (Range or chunked
manifest), the CDN/object-store contract that makes it fast, and the
hosted-service lifecycle (upload → lease → stream → writeback).

## Current state (verified)

### Controllers and machine wiring

- The canonical machine (`aero_machine::Machine`, the canonical VM core decision/0014) wires a
  deterministic Windows 7 storage topology — ICH9 AHCI for the OS disk, PIIX3
  IDE/ATAPI for install media — guarded by integration tests (see
  [topology contract](#windows-7-storage-topology-platform-abi) below).
- AHCI/IDE/ATAPI device models live in `crates/aero-devices-storage/`
  (`ahci.rs`, `ata.rs`, `ide.rs`, `atapi.rs`, `pci_ide.rs`, `pci_ahci.rs`).
- NVMe exists in `crates/aero-devices-nvme/` (admin + I/O queues, PRP/SGL,
  INTx/MSI/MSI-X; see `crates/aero-devices-nvme/tests/interrupts.rs`) but is
  **off by default for Win7** — Windows 7 has no in-box NVMe driver (Microsoft
  hotfix KB2990941 or a vendor driver would be required in-guest).
- virtio-blk (modern virtio-pci, BAR0 MMIO) is the preferred high-performance
  path once the Aero virtio Win7 drivers are installed;
  `crates/aero-virtio/` consumes `Box<dyn aero_storage::VirtualDisk>`
  directly. virtio-pci legacy/transitional (I/O-port BAR) is an optional mode,
  not part of the contract.

### Disk image formats (`crates/aero-storage/`)

All formats expose the synchronous `aero_storage::VirtualDisk` trait over a
`aero_storage::StorageBackend`:

- `RawDisk` (`disk.rs`), `AeroSparseDisk` (`sparse.rs`, magic `AEROSPAR` v1),
  `Qcow2Disk` (`qcow2.rs`, v2/v3), `VhdDisk` (`vhd.rs`, fixed/dynamic/
  differencing).
- Wrappers: `AeroCowDisk` (`cow.rs`, copy-on-write overlay),
  `BlockCachedDisk` (`cache.rs`, LRU write-back block cache with
  write-back-on-evict), `ReadOnlyDisk` / `ReadOnlyBackend`.
- `DiskImage::open_auto` **rejects** images that reference a parent (QCOW2
  backing files, VHD differencing) unless a parent is explicitly supplied via
  `open_with_parent` / `open_auto_with_parent`.
- Native file backend `StdFileBackend` (alias `FileBackend`) in
  `crates/aero-storage/src/backend.rs` backs host-side tools
  (`tools/aero-disk-convert/`, `tools/image-chunker/`).

### Browser persistence

- **OPFS SyncAccessHandle is the canonical browser backend** for the
  synchronous Rust controller path: `crates/aero-opfs/`
  (`OpfsByteStorage`, `OpfsBackend` in
  `crates/aero-opfs/src/io/storage/backends/opfs.rs`).
- **IndexedDB is async-only** and backs host-layer features (disk manager,
  import/export staging, remote chunk caches), never the boot-critical
  controller path — see [OPFS vs IndexedDB](#opfs-vs-indexeddb-sync-vs-async).
  Rust/wasm helper: `crates/st-idb/`.
- Web disk manager (create/import/convert/export/mounts, CRC32 metadata):
  `apps/web/src/storage/disk_manager.ts` + `disk_worker.ts` + `import_export.ts` +
  `metadata.ts`. OPFS layout: `aero/disks/<diskId>.<ext>` plus
  `aero/disks/metadata.json`; IndexedDB fallback database `aero-disk-manager`
  (`disks` / `mounts` / `chunks` stores).
- TS-side AEROSPAR writer/reader for OPFS caches:
  `apps/web/src/storage/opfs_sparse.ts` (same `AEROSPAR` magic).

### Remote / streaming delivery

Two delivery modes exist, both implemented:

- **HTTP Range** (single object): Rust native reference
  `aero_storage::StreamingDisk` (`crates/aero-storage/src/streaming.rs`,
  `DEFAULT_CHUNK_SIZE = 1 MiB`); TS implementation
  `apps/web/src/storage/remote_range_disk.ts` with persistent OPFS/IDB caches
  (`apps/web/src/storage/remote_cache_manager.ts`,
  `apps/web/src/storage/remote/opfs_lru_chunk_cache.ts`).
- **Chunked manifest** (no `Range` header): Rust native
  `aero_storage::ChunkedStreamingDisk`
  (`crates/aero-storage/src/chunked_streaming.rs`); TS
  `RemoteChunkedDisk` (`apps/web/src/storage/remote_chunked_disk.ts`, telemetry via
  `getTelemetrySnapshot()`); publisher/verifier CLI
  `tools/image-chunker/` (`aero-image-chunker publish|verify`).
- Authoritative chunk-size defaults: `apps/web/src/storage/chunk_sizes.ts`
  (`RANGE_STREAM_CHUNK_SIZE` = 1 MiB, `CHUNKED_DISK_CHUNK_SIZE` = 4 MiB).
- Runtime wiring for remote disks lives in
  `apps/web/src/storage/runtime_disk_worker*.ts`; a dev/telemetry panel ("Remote
  disk image (streaming)") is exposed in the canonical host (`apps/web/src/main.ts`).
- Server-side pieces: `crates/aero-storage-server/` (native HTTP image server:
  `store/` with `ImageStore`, `manifest.rs`, `local_fs.rs`; `http/` with
  `range.rs`, `chunked.rs`, `images.rs`), `crates/aero-http-range/`
  (defensive Range-header parsing), and the TS reference gateway
  `services/image-gateway/` (S3 multipart upload + CloudFront signed-cookie
  delivery, `openapi.yaml`; in-memory records, dev-stub auth).
- Conformance/perf tooling: `tools/disk-streaming-conformance/`,
  `tools/range-harness/`.

## Key contracts

### Canonical traits and layering

Source of truth for trait choice (consolidation policy absorbed from
`storage.md`):

- **Byte backend (layer 2):** `aero_storage::StorageBackend` — sync,
  byte-addressed, resizable (`crates/aero-storage/src/backend.rs`).
- **Disk formats (layer 3):** `aero_storage::VirtualDisk` — sync,
  fixed-capacity, byte addressing + sector helpers; `Send` on native, may be
  `!Send` on wasm32 (`crates/aero-storage/src/disk.rs`). New formats implement
  `VirtualDisk`; no new format-specific disk traits elsewhere.
- **Device models (layer 4):** consume `Box<dyn aero_storage::VirtualDisk>` at
  public boundaries. Crate-local backend traits that remain
  (`aero_devices::storage::DiskBackend` in `crates/devices/src/storage/mod.rs`,
  `aero_devices_storage::atapi::IsoBackend` in
  `crates/aero-devices-storage/src/atapi.rs`) are device-internal integration
  traits; adapt at the edge.
- **Adapters:** wrapper *types* live in `crates/aero-storage-adapters/`;
  trait `impl`s live in the crate that owns the trait (Rust orphan rules).
  Examples: `aero_devices::storage::AeroStorageDiskAdapter` and
  `DeviceBackendAsAeroVirtualDisk` (`crates/devices/src/storage/mod.rs`);
  `SharedDisk` bridges `firmware::bios::BlockDevice` and `VirtualDisk` so BIOS
  and PCI controllers see one disk (`crates/aero-machine/src/shared_disk.rs`).
- Read-only intent is enforced in the disk layer with `ReadOnlyDisk` /
  `ReadOnlyBackend` (base images, ISOs, remote chunked bases), with writes
  going to an overlay (`AeroCowDisk` or an OPFS sparse file).

### Windows 7 storage topology (platform ABI)

Normative; changing BDFs, attachment points, `disk_id`s, or INTx routing
requires updating the guard tests listed below.

| Purpose | Controller | PCI BDF (constant) | Attachment | Snapshot `disk_id` |
|---|---|---|---|---:|
| Primary HDD (OS disk) | ICH9 AHCI | `00:02.0` (`SATA_AHCI_ICH9`) | SATA port 0 | 0 |
| Install media (ISO) | PIIX3 IDE | `00:01.1` (`IDE_PIIX3`) | secondary channel, master (ATAPI) | 1 |
| Optional IDE ATA disk | PIIX3 IDE | `00:01.1` | primary channel, master | 2 |
| NVMe (optional) | QEMU NVMe | `00:03.0` (`NVME_CONTROLLER`) | — (off by default for Win7) | — |

- Profile constants: `crates/devices/src/pci/profile.rs` (`IDE_PIIX3`,
  `SATA_AHCI_ICH9`, `NVME_CONTROLLER`; AHCI ABAR = BAR5,
  `AHCI_ABAR_SIZE` = 0x2000). `disk_id` constants:
  `Machine::DISK_ID_PRIMARY_HDD` / `DISK_ID_INSTALL_MEDIA` /
  `DISK_ID_IDE_PRIMARY_MASTER` in `crates/aero-machine/src/lib.rs`.
- The PIIX3 ISA bridge (`ISA_PIIX3`, `00:01.0`) must also be present with the
  multi-function header bit so OS enumeration finds `00:01.1`.
- Legacy IDE compat-mode port map: primary `0x1F0..=0x1F7` / control `0x3F6`,
  secondary `0x170..=0x177` / control `0x376`; BAR1/BAR3 bases `0x3F4`/`0x374`
  (alt-status at +2), BMIDE BAR4 default `0xC000`
  (`crates/aero-devices-storage/src/pci_ide.rs`). Legacy channel interrupts
  are ISA IRQ14 (primary) / IRQ15 (secondary) — the data-plane interrupts
  software actually relies on.
- PCI INTx routing: `PciIntxRouterConfig::default()` maps Q35 root-bus
  PIRQ[A–D] → GSI[20,21,22,23]
  (`crates/aero-pci-routing/src/lib.rs`, `DEFAULT_PIRQ_TO_GSI`), with
  swizzle `PIRQ = (INTx + device) mod 4`. With INTA# on all storage
  controllers: IDE → GSI21, AHCI → GSI22, NVMe → GSI23. In legacy PIC
  mode those routes mirror to IRQ11, IRQ12, and IRQ13 respectively; the
  IOAPIC routes do not occupy those ISA GSIs.
- BIOS drive numbering: HDD0 `DL=0x80` (512-byte sectors), CD-ROM `DL=0xE0`
  (2048-byte blocks via INT 13h extensions, El Torito no-emulation);
  `crates/firmware/src/bios/mod.rs`. An optional "CD-first when present"
  policy (`Machine::configure_win7_install_boot` /
  `MachineConfig::win7_install_defaults`) keeps `boot_drive=0x80` as fallback;
  `Machine::active_boot_device()` reports what firmware actually booted.
- Construction helpers: `MachineConfig::win7_storage_defaults` /
  `win7_storage(...)` / `win7_install_defaults(...)`,
  `Machine::new_with_win7_storage` / `new_with_win7_install`,
  `Machine::set_boot_drive` + `Machine::reset()`
  (`crates/aero-machine/src/lib.rs`);
  `PcPlatform::new_with_win7_storage` /
  `new_with_windows7_storage_topology` in `crates/aero-pc-platform/`.
- Browser runtime: only `vmRuntime=machine` (`?vm=machine`) runs this full
  topology (via `aero_machine::Machine` in the CPU worker); the legacy runtime
  does not implement the Win7 AHCI/IDE bring-up path
  (`apps/web/src/runtime/coordinator.ts`). Boot introspection for automation:
  `machine.boot_drive()` / `boot_from_cd_if_present()` / `cd_boot_drive()` /
  `active_boot_device()` on the wasm `Machine`, and
  `window.aero.debug.getMachineCpuActiveBootDevice()` /
  `getMachineCpuBootConfig()` on the main thread (both may return `null`
  during transitions).

Guard tests: `crates/devices/tests/win7_storage_topology.rs`,
`crates/aero-pc-platform/tests/pc_platform_win7_storage.rs` and
`windows7_storage_topology.rs`,
`crates/aero-machine/tests/machine_win7_storage_topology.rs`,
`win7_storage_topology.rs`, `machine_win7_storage_helper.rs`,
`machine_win7_storage.rs`, `machine_disk_overlays_snapshot.rs`,
`machine_storage_snapshot_roundtrip.rs`.

### ATA/ATAPI command support

AHCI (`crates/aero-devices-storage/src/ahci.rs` + ATA drive model in
`ata.rs`): `IDENTIFY DEVICE` (0xEC), `READ/WRITE DMA` (0xC8/0xCA),
`READ/WRITE SECTORS` (0x20/0x30), `READ/WRITE DMA EXT` (0x25/0x35),
`READ/WRITE SECTORS EXT` (0x24/0x34), `FLUSH CACHE` (0xE7) / `FLUSH CACHE
EXT` (0xEA), `SET FEATURES` (0xEF; write-cache enable 0x02 / disable 0x82,
other subcommands succeed as no-ops). All data transfer is PRDT DMA — even
for the "PIO" opcodes (there is no PIO data register in AHCI). Port reset
(COMRESET via `PxSCTL.DET`) is modelled synchronously: link-down while
asserted, immediate link-up + `PxIS.DHRS` on deassert; `DET=4`/`DET=2` treated
as PHY-offline until `DET=0`.

IDE (`crates/aero-devices-storage/src/ide.rs`) implements the same core ATA
set with true PIO data-port semantics (needed by BIOS/early boot), plus bus
master DMA. ATAPI CD-ROM (`atapi.rs`) serves the usual packet commands
(INQUIRY, READ(10)/(12), READ CAPACITY, READ TOC, MODE SENSE, TEST UNIT
READY, START STOP UNIT) against an `IsoBackend`.

### AeroSparse on-disk format (`AEROSPAR`)

`crates/aero-storage/src/sparse.rs`: 8-byte magic `AEROSPAR`, version 1,
64-byte header, then a little-endian `u64` allocation table (physical block
offset or 0 = unallocated → reads as zeros), then fixed-size data blocks.
`block_size_bytes` must be a power-of-two multiple of 512, capped at 64 MiB;
~1 MiB is the common choice. Supports best-effort `discard_range` for fully
covered blocks.

### Snapshot DISKS contract

Snapshots store disk/ISO *references* (`aero_snapshot::DiskOverlayRef {
disk_id, base_image, overlay_image }`) in the `DISKS` section
(`crates/aero-snapshot/`); `base_image`/`overlay_image` are opaque host
identifiers (empty = not configured). In the browser machine runtime they are
OPFS-root-relative paths without `..` segments. On restore, controller
snapshots deliberately drop host backends: the host must re-open the
referenced images and re-attach them at the canonical attachment points
before resuming. The `disk_id` mapping is platform ABI (guard:
`crates/aero-machine/tests/machine_disk_overlays_snapshot.rs`).

### Disk-bytes HTTP contract (Range mode)

Normative requirements for any endpoint `StreamingDisk` reads from
(absorbed from `storage.md`):

- `HEAD` returns `200` + `Content-Length` + `Accept-Ranges: bytes` +
  `ETag`/`Last-Modified`. Clients use it for size + cache validator.
- Single-range `GET` (`Range: bytes=<start>-<end>`) → `206 Partial Content` +
  `Content-Range: bytes <start>-<end>/<total>` + `Content-Length` of the
  slice. Unsatisfiable range → `416` + `Content-Range: bytes */<total>`.
  Multi-range requests are rejected (`416`/`400`), never silently ignored —
  a full `200` to a ranged request corrupts the virtual disk.
- No transforms: `Content-Encoding: identity` (or absent) and
  `Cache-Control` must include `no-transform`; Aero clients reject `206`
  responses without it.
- Strong `ETag` that changes with the bytes (content hash / version id /
  `{diskId}:{generation}`); `If-Range` resume semantics. Treat S3 ETags as
  version validators, not integrity hashes (multipart ETags encode part
  counts).
- Resource is immutable for the lease lifetime; new bytes ⇒ new URL/ETag.
  Versioned keys (`/images/<imageId>/<version>/disk.img`) with a short-TTL
  `latest.json` pointer instead of in-place overwrite.
- CORS (when cross-origin): preflight must allow `GET, HEAD, OPTIONS` and
  headers `Range, If-Range, If-None-Match, If-Modified-Since, Authorization`;
  `GET`/`HEAD`/`416` responses must expose `Accept-Ranges, Content-Range,
  Content-Length, ETag, Content-Encoding` (`Last-Modified` is safelisted).
  `Access-Control-Max-Age: 600+` recommended. Credentialed mode requires an
  explicit origin (never `*`) + `Access-Control-Allow-Credentials: true` +
  `Vary: Origin`.
- Cross-origin isolation (the cross-origin isolation decision): disk responses must be CORS-enabled
  and/or carry a compatible `Cross-Origin-Resource-Policy`; prefer
  `COEP: require-corp` when using cookie-based cross-origin auth.
- Prefer **same-origin** disk endpoints (path-based CDN routing) to avoid
  preflight entirely.

Auth model: a short-lived **disk access lease** separates the control plane
(session auth, authorization, lease issuance) from the data plane
(high-volume Range reads). Lease presented as signed URL, signed cookie, or
`Authorization: Bearer`; refreshed proactively before expiry and reactively
on `401`/`403` (refresh once, retry once). Lease URLs/tokens are secrets:
never persisted to OPFS/IndexedDB/localStorage or logs
(`apps/web/src/storage/disk_access_lease.ts` implements the refreshable lease).

### Chunked disk image format (no-Range)

Schema `aero.chunked-disk-image.v1` (absorbed from
`../specs/chunked-disk-image-format.md`; implemented by
`crates/aero-storage/src/chunked_streaming.rs`,
`apps/web/src/storage/remote_chunked_disk.ts`, published by
`tools/image-chunker/`):

- Layout: `images/<imageId>/<version>/manifest.json` +
  `chunks/<zero-padded index>.bin`. Chunks are the **logical disk byte
  stream** (what the guest sees), not container file bytes.
- Manifest: `totalSize`, `chunkSize` (default 4 MiB; multiple of 512),
  `chunkCount`, `version`, `mimeType`, `chunkIndexWidth` (recommended 8);
  optional per-chunk `size`/`sha256`. Client-side defensive limits in the
  reference implementations: `chunkSize` ≤ 64 MiB, `chunkCount` ≤ 500,000,
  `chunkIndexWidth` ≤ 32, manifest ≤ 64 MiB.
- `version` derives from the full logical byte stream so chunk URLs are
  immutable → `Cache-Control: public, max-age=31536000, immutable,
  no-transform` on both manifest and chunks; `Content-Encoding` must be
  absent/`identity` (reference clients treat violations as protocol errors).
- Read mapping: `chunkIndex = floor(offset / chunkSize)`; fetch whole chunks
  with plain `GET` (no `Range` ⇒ no CORS preflight), track downloaded chunks
  in chunk units, cache persistently (OPFS).
- Publish pipeline: chunks first, manifest last (manifest is authoritative);
  `aero-image-chunker verify` re-downloads and validates end-to-end (S3,
  `--manifest-url`, or `--manifest-file`), fail-fast, `--chunk-sample N` for
  smoke checks. Images with parents (QCOW2 backing / VHD differencing) must
  be flattened before chunking.

## OPFS vs IndexedDB (sync vs async)

The Rust controller stack is synchronous; IndexedDB is not. A synchronous
wrapper around IndexedDB **deadlocks** in the same worker: IDB completions
are delivered on that worker's event loop, which any sync wait (spin,
`Atomics.wait`, `block_on`) blocks.

- **Current policy (option A):** OPFS `FileSystemSyncAccessHandle`
  (worker-only) is a hard requirement for the Rust controller path; the web
  runtime gates on it (`apps/web/src/runtime/coordinator.ts`,
  `apps/web/src/platform/features.ts`, mirrored in `src/platform/features.ts`).
  IndexedDB remains for the async host layer: disk manager
  (`apps/web/src/storage/indexeddb.ts`), import/export staging, remote chunk
  caches, benchmarks; Rust helper crate `crates/st-idb/`
  (`st_idb::io::storage::DiskBackend`).
- Footgun: `crates/aero-opfs` can construct an `OpfsIndexedDbBackend`
  fallback, but it is async-only and cannot back
  `aero_storage::VirtualDisk`; doc-comments in
  `crates/aero-opfs/src/io/storage/backends/opfs.rs` say so.
- OPFS SyncAccessHandles are **exclusive per file** — only one handle per
  OPFS file at a time, so a disk image must be opened at most once
  concurrently (in `vmRuntime=machine`, only the worker owning the canonical
  `api.Machine` opens OPFS-backed disks).
- **[aspirational] option C:** if IndexedDB must ever back the sync
  controller path, run IDB on a separate worker and expose a sync facade via
  a shared-memory RPC (`Atomics.wait`), reusing the AIPC ring primitives
  (`apps/web/src/io/ipc/aero_ipc_io.ts`) and implementing the canonical
  `StorageBackend`/`VirtualDisk` on the RPC client. Not scheduled.

## Configuration & knobs

- Chunk sizes: `apps/web/src/storage/chunk_sizes.ts` (1 MiB Range / 4 MiB
  chunked). Tuning guidance: keep power-of-two multiples of 512; smaller
  chunks cut over-fetch, larger chunks cut request rate.
- Remote disk caches: LRU eviction with configurable MiB limit (0 =
  unbounded) in `apps/web/src/storage/remote/opfs_lru_chunk_cache.ts`; cache
  identity can be pinned separately from the URL (`cacheImageId`,
  `cacheVersion`) to survive ephemeral auth query params.
- Credentials mode (`same-origin` / `include` / `omit`) for cookie-auth or
  credentialed CORS setups, on the remote-disk open path.
- Cache validators: ETag/Last-Modified changes must invalidate locally cached
  chunks.

## CDN / ops guidance (deployment contracts)

Absorbed from `storage.md` and
`storage.md`. Reference IaC in the repo:
`infra/local-object-store/` (MinIO + proxy for local dev) and
`infra/aws-s3-cloudfront-range/` (S3 + CloudFront tuned for Range + CORS).
> Working-tree note: `infra/` and `deploy/` are present in `HEAD` but
> currently deleted (uncommitted) in this checkout — treat those two paths as
> in-flux.

- Baseline: private bucket + Origin Access Control, serve only through the
  CDN; immutable versioned keys; long TTLs (`public, max-age=31536000,
  immutable, no-transform`); minimal cache key (path + version) — never vary
  on `Range`, cookies, or auth headers.
- CloudFront caches `206` range responses and can serve later ranges from
  edge cache; do not put `Range` in the cache key. Max cacheable response
  size is 50 GB per GET — for larger images use range-only access or chunk
  objects. Origin must set `Content-Length` on `206` (a chunked
  `Transfer-Encoding` origin response makes CloudFront return the whole
  object).
- Cloudflare: edge cacheable size limits are 512 MB (Free/Pro/Business) /
  5 GB (Enterprise) — chunk objects are effectively mandatory there; if the
  origin omits `Content-Length`, Cloudflare silently falls back to full-`200`
  responses. R2's `r2.dev` public URL is non-production; use a custom domain.
- Self-hosted Nginx: use the `slice` module so the cache stores fixed-size
  slices keyed by slice range, not arbitrary viewer ranges.
- Validation checklist: `curl` the CDN (not origin) for HEAD size/validators,
  `206` + `Content-Range` correctness, `X-Cache`/`Age` hit behavior on a
  repeated range (same connection or pinned edge IP — CloudFront POPs don't
  share caches), and watch origin request counts. Automated:
  `tools/disk-streaming-conformance/`, `tools/range-harness/`.
- Vendor behavior above is from published docs/field observation, not from
  infra in this repo — validate per deployment. **[unverified]** against live
  infrastructure.

## Hosted service model

Reference implementation: `services/image-gateway/` (S3 multipart upload with
presigned PUT URLs → stable CloudFront URL + signed cookies → browser streams
via Range; image records in memory, auth is a dev stub — it is a reference,
not a product).

The lifecycle contract (absorbed from
`storage.md`):

- Users bring their own media; the service does not provide Windows images.
  Uploads are private by default.
- States: `uploading → processing → ready` (or `failed`/`deleted`); leases
  are minted only for `ready` images. Canonical stored formats: ISO as-is
  (read-only), raw disk blobs; QCOW2/VHD imports are converted to raw.
- Scopes: `disk:read` / `disk:write` / `disk:upload` / `disk:delete` /
  `disk:share`. Management plane on user sessions; data plane on short-lived
  per-image leases (minutes), renewable.
- Data plane rejects expired or scope-mismatched leases (`401`/`403`) and
  must satisfy the Range contract above.
- Writeback strategies, in increasing server complexity:
  1. **Remote read-only base + local OPFS COW delta** — the implemented
     default (only `disk:read` needed; no cross-device resume).
  2. Remote base + remote per-user delta (sparse fixed-size blocks;
     `disk:write` scoped to the delta). **[aspirational]**
  3. Fully remote read-write block API. **[aspirational]**
- Sharing (ACL grants + hashed, revocable, time-limited share links), quotas,
  and abuse controls are design intent. **[aspirational]**

## Open issues & debt

- **Boot gap**: no verified Win7 boot-to-desktop (see
  `wiki/state/repo-state-and-structure.md` §2); the storage stack's end-to-end
  value is unproven past BIOS/El Torito.
- **Legacy sparse format**: `AEROSPRS` (4 KiB header plus journal) survives in
  [`crates/aero-storage/src/sparse_v1.rs`](../../crates/aero-storage/src/sparse_v1.rs)
  only to open old images; new work uses `AEROSPAR`. The offline converter is
  the `aerosparse_convert` binary of the same crate.
- **NVMe for Win7** needs an in-guest driver; keep off by default.
- **Perf targets unmeasured**: the old subsystem doc's sequential/random IOPS
  table was aspirational and is dropped; real numbers belong in `bench/`.
- **TS test suite** not run since the sprint freeze (state doc §3), so the
  `apps/web/src/storage/` surface is compile-checked at best. **[unverified]**

## Legacy / historical

- The `AEROSPRS` format (above).
- The legacy browser runtime (`vmRuntime=legacy`) lacks the Win7 storage
  topology; kept only for non-Win7 experiments.
- Sprint-era task references in the absorbed docs (e.g. the IndexedDB fallback
  task in `instructions/io-storage.md`) are historical; this page is the
  current policy.

## Pointers

- State/structure: `wiki/state/repo-state-and-structure.md`
- Snapshot format: `wiki/areas/platform-and-firmware.md`
- El Torito CD boot: `wiki/areas/platform-and-firmware.md`
- Win7 virtio driver contract: `wiki/areas/windows-drivers.md`
 
- BIOS: `crates/firmware/`, machine wiring: `crates/aero-machine/`

## Device-model fixes applied (2026-07-26/27)

See `wiki/notes/bring-up-findings-divergence-and-sight-audits.md` §6 for full details.

- **AHCI**: removed PRD 512-byte alignment gate (was dropping valid scatter-gather
  entries); pinned `GHC.AE` always-on (ICH9 RO-1); added SET FEATURES 0x03
  handling for transfer-mode select.
- **IDE/ATAPI**: ATAPI signature (0x14/0xEB) now presented on SRST reset (was
  all-zero → CD-ROM undetectable via SRST enumeration); IDENTIFY PACKET DEVICE
  word 0 no longer advertises 16-byte packets (was wrong bit); added DMA mode
  words 53/63/88; ATAPI devices now accept SET FEATURES (was abort → PIO-only).
- **NVMe**: Get Log Page (0x02) and AER (0x0C) handled (were INVALID_OPCODE →
  driver init abort); CAP.TO set to 4s (was 0 → spurious timeout).
- **virtio-blk**: removed DISCARD/WRITE_ZEROES feature bits per Win7 contract
  §3.1.3; zeroed config fields 0x24-0x38 per §3.1.4.
- **virtio-input**: fixed event loss on malformed descriptor chains (peek before
  pop pattern).
- **virtio-snd**: fixed `virtio_snd_pcm_info` struct layout (36 bytes with `le64
  hdr`, was 32 bytes with shifted offsets).
- **PCI config**: undefined BARs return 0 on sizing probe (were returning
  0xFFFFFFFF → phantom IO BARs).

## Storage Subsystem

### Overview

Windows 7 requires significant storage (15-40GB installed). The storage subsystem must efficiently emulate disk controllers while using browser storage APIs that have their own constraints.

While AHCI provides out-of-the-box Windows compatibility, **virtio-blk** is the preferred high-performance path once virtio drivers are available. Under Aero’s Windows 7 virtio contract ([`AERO-W7-VIRTIO` v1](../specs/windows7-virtio-driver-contract.md)), virtio devices are exposed as **virtio-pci modern-only** (virtio 1.0+) via PCI vendor-specific capabilities and a single **BAR0 MMIO** register region.

Compatibility note: adding virtio-pci legacy/transitional (I/O port BAR) support may be desirable for some **upstream virtio-win** driver bundles, but it is **not required** by the Aero contract and is treated as an optional mode. See: [`16-virtio-pci-legacy-transitional.md`](windows-drivers.md)

---

### Canonical Windows 7 storage topology (PCI + media attachment)

For Windows 7 boot/install, Aero uses a **deterministic, compatibility-first storage topology**:

- **ICH9 AHCI** for the primary HDD (Windows installs/boots from this disk)
- **PIIX3 IDE + ATAPI** for the CD-ROM (Windows install ISO / driver ISOs)

The canonical PCI BDF assignments, port/drive mapping, BIOS boot flows, and INTx→GSI routing are
defined in:

- [`05-storage-topology-win7.md`](storage.md)

This topology is treated as part of the platform ABI: drift should be caught by unit tests.

Browser/runtime note: the canonical full-system browser runtime (`vmRuntime=machine`, i.e.
`?vm=machine`) runs `api.Machine` (backed by `aero_machine::Machine`) and uses this topology. The
legacy browser runtime does not implement the full AHCI/IDE Win7 bring-up path.

---

### Storage Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Storage Stack                                 │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Windows 7                                                       │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  File System (NTFS)                                      │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  Volume Manager (volmgr.sys)                             │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  Disk Class Driver (disk.sys)                            │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  Storage miniport (AHCI: msahci.sys on Win7)             │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
└───────┼─────────────────────────────────────────────────────────┘
        │  ◄── Emulation Boundary
        ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Aero Storage Emulation                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  AHCI Controller Emulation                               │    │
│  │    - HBA Memory Registers                                │    │
│  │    - Port Registers                                      │    │
│  │    - Command List Processing                             │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │  Virtual Disk Layer                                      │    │
│  │    - Sector Read/Write                                   │    │
│  │    - DMA Transfers                                       │    │
│  └─────────────────────────────────────────────────────────┘    │
│       │                                                          │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐    │
│  │  OPFS Backend  │  │IndexedDB Cache │  │  Remote API    │    │
│  │  (large files) │  │ (async cache)  │  │  (streaming)   │    │
│  └────────────────┘  └────────────────┘  └────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

> Important: IndexedDB is **async-only** in the browser. The canonical Rust disk/controller stack
> (`aero-storage::{StorageBackend, VirtualDisk}` + `aero-devices-storage` AHCI/IDE) is
> **synchronous**, so IndexedDB cannot be used directly from that path without a cross-worker
> design. See: [`19-indexeddb-storage-story.md`](storage.md) and the canonical
> trait guidance in [`20-storage-trait-consolidation.md`](storage.md).

---

### AHCI Controller Emulation

#### AHCI Overview

AHCI (Advanced Host Controller Interface) is the standard SATA controller interface used by Windows 7.

```rust
pub struct AhciController {
    // HBA Memory Registers
    hba: HbaMemory,
    
    // Ports (up to 32)
    ports: [AhciPort; 32],
    
    // Connected drives
    drives: Vec<VirtualDrive>,
    
    // IRQ state
    irq_pending: bool,
}

#[repr(C)]
pub struct HbaMemory {
    cap: u32,        // Host Capabilities
    ghc: u32,        // Global Host Control
    is: u32,         // Interrupt Status
    pi: u32,         // Ports Implemented
    vs: u32,         // Version
    ccc_ctl: u32,    // Command Completion Coalescing Control
    ccc_ports: u32,  // Command Completion Coalescing Ports
    em_loc: u32,     // Enclosure Management Location
    em_ctl: u32,     // Enclosure Management Control
    cap2: u32,       // Extended Capabilities
    bohc: u32,       // BIOS/OS Handoff Control
}

#[repr(C)]
pub struct PortRegisters {
    clb: u64,        // Command List Base Address
    fb: u64,         // FIS Base Address
    is: u32,         // Interrupt Status
    ie: u32,         // Interrupt Enable
    cmd: u32,        // Command and Status
    reserved: u32,
    tfd: u32,        // Task File Data
    sig: u32,        // Signature
    ssts: u32,       // SATA Status
    sctl: u32,       // SATA Control
    serr: u32,       // SATA Error
    sact: u32,       // SATA Active
    ci: u32,         // Command Issue
    sntf: u32,       // SATA Notification
    fbs: u32,        // FIS-based Switching Control
}
```

#### Command Processing

```rust
impl AhciController {
    pub fn process_port(&mut self, port_num: usize, memory: &mut MemoryBus) {
        let port = &mut self.ports[port_num];
        
        // Check if commands are issued
        while port.ci != 0 {
            // Find the lowest set bit (next command slot)
            let slot = port.ci.trailing_zeros() as usize;
            
            // Read command header from guest memory
            let cmd_header_addr = port.clb + (slot * 32) as u64;
            let cmd_header = self.read_command_header(memory, cmd_header_addr);
            
            // Read command table
            let cmd_table_addr = cmd_header.ctba;
            let cmd_table = self.read_command_table(memory, cmd_table_addr);
            
            // Process the command FIS
            self.process_command_fis(port_num, &cmd_header, &cmd_table, memory);
            
            // Clear command slot
            port.ci &= !(1 << slot);
            
            // Signal completion
            port.is |= AHCI_PORT_IS_DHRS;  // Device to Host Register FIS
            
            if port.ie & AHCI_PORT_IE_DHRE != 0 {
                self.raise_irq();
            }
        }
    }
    
    fn process_command_fis(
        &mut self,
        port: usize,
        header: &CommandHeader,
        table: &CommandTable,
        memory: &mut MemoryBus,
    ) {
        let fis = &table.cfis;
        
        match fis.command {
            ATA_CMD_IDENTIFY => {
                self.do_identify(port, header, table, memory);
            }

            // Note: Even for "PIO" opcodes (READ/WRITE SECTORS), AHCI still uses PRDT DMA.
            ATA_CMD_READ_DMA | ATA_CMD_READ_SECTORS => {
                let lba = self.extract_lba28(fis)?;
                let count = self.extract_sector_count_28(fis);
                self.do_read_dma(port, lba, count, header, table, memory);
            }
            ATA_CMD_READ_DMA_EXT | ATA_CMD_READ_SECTORS_EXT => {
                let lba = self.extract_lba48(fis);
                let count = self.extract_sector_count(fis);
                self.do_read_dma(port, lba, count, header, table, memory);
            }
            ATA_CMD_WRITE_DMA | ATA_CMD_WRITE_SECTORS => {
                let lba = self.extract_lba28(fis)?;
                let count = self.extract_sector_count_28(fis);
                self.do_write_dma(port, lba, count, header, table, memory);
            }
            ATA_CMD_WRITE_DMA_EXT | ATA_CMD_WRITE_SECTORS_EXT => {
                let lba = self.extract_lba48(fis);
                let count = self.extract_sector_count(fis);
                self.do_write_dma(port, lba, count, header, table, memory);
            }

            ATA_CMD_FLUSH_CACHE | ATA_CMD_FLUSH_CACHE_EXT => {
                self.do_flush(port);
            }
            ATA_CMD_SET_FEATURES => {
                self.do_set_features(port, fis);
            }

            _ => {
                // Unsupported commands should complete with an ATA abort error (TFES/DHRS in AHCI).
                self.abort_command(port, fis.command);
            }
        }
    }
    
    fn do_read_dma(
        &mut self,
        port: usize,
        lba: u64,
        sector_count: u32,
        header: &CommandHeader,
        table: &CommandTable,
        memory: &mut MemoryBus,
    ) {
        let drive = &self.drives[port];
        let sector_size = drive.sector_size;
        
        // Read from virtual disk.
        //
        // NOTE: This is a simplified example. The real implementation avoids allocating a full
        // contiguous buffer for large transfers and instead streams through the PRDT
        // scatter/gather list in bounded chunks.
        let mut data = vec![0u8; (sector_count as usize) * sector_size];
        drive.read_sectors(lba, &mut data);
        
        // DMA transfer to guest memory using PRD table
        let mut offset = 0;
        for prd in &table.prdt {
            let bytes_to_copy = ((prd.dbc & 0x3FFFFF) + 1) as usize;
            let dest_addr = prd.dba;
            
            memory.write_physical_bulk(dest_addr, &data[offset..offset + bytes_to_copy]);
            offset += bytes_to_copy;
            
            if offset >= data.len() {
                break;
            }
        }
    }
}
```

#### Supported ATA commands (AHCI, current implementation)

The canonical AHCI implementation used by the Rust storage controller stack lives in:

- [`crates/aero-devices-storage/src/ahci.rs`](../../crates/aero-devices-storage/src/ahci.rs)
- ATA drive model + IDENTIFY data: [`crates/aero-devices-storage/src/ata.rs`](../../crates/aero-devices-storage/src/ata.rs).
  IDENTIFY matches QEMU/ATAPI field-validity: word 53 bits 0–2, current CHS in
  54–58, IORDY in word 49, word 47 `0x8010`, and command-set words 83/84/87 with
  bit 14 set / bit 15 clear (otherwise Win7 classpnp can leave
  `Tracks*Sectors=0` and never publish `MediaType=FixedMedia`).

As of the current implementation, the AHCI path supports the following ATA commands:

- `IDENTIFY DEVICE` (`0xEC`)
- `READ DMA` (28-bit) (`0xC8`)
- `WRITE DMA` (28-bit) (`0xCA`)
- `READ SECTORS` (28-bit) (`0x20`)
- `WRITE SECTORS` (28-bit) (`0x30`)
- `READ DMA EXT` (48-bit) (`0x25`)
- `WRITE DMA EXT` (48-bit) (`0x35`)
- `READ SECTORS EXT` (48-bit) (`0x24`)
- `WRITE SECTORS EXT` (48-bit) (`0x34`)
- `FLUSH CACHE` (`0xE7`) and `FLUSH CACHE EXT` (`0xEA`)
- `SET FEATURES` (`0xEF`)
  - `0x02` = enable write cache
  - `0x82` = disable write cache
  - Other subcommands currently complete successfully but are treated as no-ops.

Notes:

- These commands are executed via AHCI DMA (PRDT scatter/gather) against an `AtaDrive` backed by an
  `aero_storage::VirtualDisk`.
- Even for *PIO opcodes* like `READ SECTORS` / `WRITE SECTORS`, AHCI still performs data transfer via
  PRDT-based DMA (there is no PIO data register in the AHCI programming model).
- The legacy IDE controller model is still important because it implements *actual* PIO data port
  semantics (I/O port reads/writes to the ATA DATA register), which is required for BIOS/early-boot
  compatibility. See: [`crates/aero-devices-storage/src/ide.rs`](../../crates/aero-devices-storage/src/ide.rs).

For reference, the current IDE (PIO) ATA command support includes:

- `IDENTIFY DEVICE` (`0xEC`)
- `READ SECTORS` (28-bit PIO) (`0x20`)
- `WRITE SECTORS` (28-bit PIO) (`0x30`)
- `READ SECTORS EXT` (48-bit PIO) (`0x24`)
- `WRITE SECTORS EXT` (48-bit PIO) (`0x34`)
- `FLUSH CACHE` (`0xE7`) and `FLUSH CACHE EXT` (`0xEA`)
- `SET FEATURES` (`0xEF`) (write cache enable/disable, as above)
  - Other subcommands currently complete successfully but are treated as no-ops.

#### AHCI port reset / COMRESET handling (PxSCTL.DET)

Real AHCI drivers (including Windows 7’s in-box `msahci.sys`) commonly perform a SATA link reset by
toggling `PxSCTL.DET`:

1. `PxSCTL.DET = 1` (COMRESET)
2. `PxSCTL.DET = 0` (idle)
3. Poll `PxSSTS` and `PxTFD` until the link/device are ready, then issue commands.

Aero models this flow in a minimal, synchronous way:

- While COMRESET is asserted (`DET=1`), the port reports a link-down/resetting view, clears
  in-flight state (`PxCI`/`PxSACT`/`PxIS`), resets `PxSERR` to a basic diagnostic bit, and sets
  `PxTFD.BSY` when a drive is present.
- When COMRESET is deasserted (`DET` transitions `1 → 0`), link-up completion happens immediately
  and the port restores `PxSSTS`/`PxSIG` and asserts `PxIS.DHRS` for an attached drive.
- `DET=4` (and the commonly-seen `DET=2`) are treated as “port disable / PHY offline” until the
  guest writes `DET=0` again.

See the `PxSCTL` handling and link-ready gating in
[`crates/aero-devices-storage/src/ahci.rs`](../../crates/aero-devices-storage/src/ahci.rs).

---

### Virtual Disk Implementation

#### Disk Image Formats

```rust
pub enum DiskFormat {
    Raw,              // Direct sector mapping
    Qcow2,            // QEMU Copy-on-Write v2
    Vhd,              // Microsoft Virtual Hard Disk
    AeroSparse,       // Aero sparse format (`AEROSPAR`)
}

pub struct VirtualDrive {
    format: DiskFormat,
    sector_size: usize,
    total_sectors: u64,
    backend: Box<dyn DiskBackend>,
    write_cache: WriteCache,
}

pub trait DiskBackend: Send + Sync {
    fn read_sectors(&self, lba: u64, buffer: &mut [u8]) -> Result<()>;
    fn write_sectors(&mut self, lba: u64, buffer: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
    fn capacity(&self) -> u64;
}
```

Note: in the current codebase, the canonical traits live in `crates/aero-storage/`:
`aero_storage::VirtualDisk` (byte-addressed with sector helpers) and
`aero_storage::StorageBackend` (resizable byte storage). Device models that use their own
disk traits typically consume an `aero_storage::VirtualDisk` via `crates/aero-storage-adapters/`
(or accept a boxed `VirtualDisk` directly, as in `aero-virtio`’s virtio-blk device model).
See also: [`20-storage-trait-consolidation.md`](storage.md).

**Implementation status (reference implementation):**
The canonical Rust disk image formats live in `crates/aero-storage/` and currently support:

- **Raw** (`aero_storage::RawDisk`) - direct mapping of bytes to sectors.
- **Aero Sparse (`AEROSPAR`, v1; `aero_storage::AeroSparseDisk`)** (Aero-specific sparse format for large virtual disks).\
  Implementation: [`crates/aero-storage/src/sparse.rs`](../../crates/aero-storage/src/sparse.rs). See also:
  [`20-storage-trait-consolidation.md`](storage.md).
- **QCOW2 v2/v3** (common unencrypted, uncompressed images; backing files require an explicit parent disk when opened).
- **VHD fixed, dynamic, and differencing** (unallocated blocks read as zeros; writes allocate blocks and update BAT/bitmap; differencing disks require an explicit parent when opened).
- **Copy-on-write overlays** (`aero_storage::AeroCowDisk`) - writable overlay on top of a base disk.
- **Write-back block caching** (`aero_storage::BlockCachedDisk`) - performance wrapper for small random I/O.

For container formats that can reference a base layer (QCOW2 backing files, VHD differencing disks),
`DiskImage::open_auto` intentionally rejects the image unless you explicitly supply a parent disk.
To open these, use:

- `aero_storage::DiskImage::open_with_parent` / `aero_storage::DiskImage::open_auto_with_parent`, or
- format-specific helpers like `aero_storage::{Qcow2Disk, VhdDisk}::open_with_parent`.

#### Block cache (`aero_storage::BlockCachedDisk`)

For synchronous controller paths, it is common to place a block cache in front of the “real” disk
image backend to reduce the overhead of many small sector operations.

`aero_storage::BlockCachedDisk` is a fixed-block-size **LRU write-back cache** wrapper around an
`aero_storage::VirtualDisk`:

- Implementation: [`crates/aero-storage/src/cache.rs`](../../crates/aero-storage/src/cache.rs)
- Property-test coverage (including write-back-on-evict): [`crates/aero-storage/tests/prop_storage.rs`](../../crates/aero-storage/tests/prop_storage.rs)

Robustness notes (current behavior):

- Dirty blocks are written back on `flush()` **and** when evicted due to LRU pressure (to avoid
  dropping modified data under memory pressure).
- If write-back fails during eviction, the entry is reinserted into the cache and the eviction is
  aborted (to avoid losing dirty data on transient backend errors).
- Write-back clamps the final partial block to disk capacity and uses checked offset arithmetic to
  avoid panics on edge cases (e.g. near-`u64::MAX` offsets).

#### OPFS Backend

In the repo, the OPFS backend is implemented in Rust/wasm32 in `crates/aero-opfs`
(e.g. `aero_opfs::OpfsBackend` / `aero_opfs::OpfsByteStorage`).

For the Rust controller path, **OPFS SyncAccessHandle is the expected backend** because it is
actually synchronous inside a Worker and can implement `aero_storage::StorageBackend` /
`aero_storage::VirtualDisk`. IndexedDB is async-only; see:
[`19-indexeddb-storage-story.md`](storage.md) and
[`20-storage-trait-consolidation.md`](storage.md).

Important caveat: OPFS `FileSystemSyncAccessHandle` is **exclusive per file**. Browsers generally
allow only **one** SyncAccessHandle to be open for a given OPFS file at a time; attempting to open a
second handle (even from another Worker in the same origin) typically fails with an
`InvalidStateError` due to the file lock.

The web runtime must therefore ensure each disk image file is opened at most once concurrently. In
`vmRuntime=machine` mode, only the worker that owns the canonical `api.Machine` opens OPFS-backed
disks; other workers avoid opening competing handles.

The snippet below is illustrative; see `crates/aero-opfs` for the current implementation.

```rust
pub struct OpfsBackend {
    file_handle: FileSystemFileHandle,
    sync_handle: FileSystemSyncAccessHandle,
    sector_size: usize,
}

impl OpfsBackend {
    pub async fn open(path: &str) -> Result<Self> {
        let root = navigator_storage_get_directory().await?;
        
        let file_handle = root
            .get_file_handle(path, GetFileHandleOptions { create: true })
            .await?;
        
        // Get synchronous access handle for performance
        let sync_handle = file_handle.create_sync_access_handle().await?;
        
        Ok(Self {
            file_handle,
            sync_handle,
            sector_size: 512,
        })
    }
}

impl DiskBackend for OpfsBackend {
    fn read_sectors(&self, lba: u64, buffer: &mut [u8]) -> Result<()> {
        let offset = lba * self.sector_size as u64;
        self.sync_handle.read(buffer, offset)?;
        Ok(())
    }
    
    fn write_sectors(&mut self, lba: u64, buffer: &[u8]) -> Result<()> {
        let offset = lba * self.sector_size as u64;
        self.sync_handle.write(buffer, offset)?;
        Ok(())
    }
    
    fn flush(&mut self) -> Result<()> {
        self.sync_handle.flush()?;
        Ok(())
    }
}
```

#### Native file backend (non-wasm)

For **host-side tooling** (image conversion, chunk publishing, offline verification) and native
tests, prefer a native `aero_storage::StorageBackend` implementation backed by the local
filesystem (e.g. `aero_storage::FileBackend` / `aero_storage::StdFileBackend`).

This keeps the layering consistent with [`20-storage-trait-consolidation.md`](storage.md):
tools can reuse `aero-storage` disk formats (`DiskImage::open_auto`) and wrappers without
re-implementing format parsing against `std::fs::File`.

#### Sector Cache (IndexedDB)

IndexedDB can still be useful for *async* host-layer caching and disk management, but it is not a
drop-in backend for the synchronous Rust controller stack. See:
[`19-indexeddb-storage-story.md`](storage.md) and
[`20-storage-trait-consolidation.md`](storage.md).

In this repo, the Rust async IndexedDB block store lives in `crates/st-idb`, and the TypeScript
host-side storage utilities (disk manager/import/export, remote disk caching) live under
`apps/web/src/storage/`.

```rust
pub struct SectorCache {
    db: IdbDatabase,
    cache: LruCache<u64, Vec<u8>>,
    dirty_sectors: HashSet<u64>,
    max_cached: usize,
}

impl SectorCache {
    pub async fn new(db_name: &str, max_sectors: usize) -> Result<Self> {
        let db = IdbDatabase::open(db_name, 1, |db| {
            db.create_object_store("sectors", ObjectStoreOptions {
                key_path: Some("lba"),
            });
        }).await?;
        
        Ok(Self {
            db,
            cache: LruCache::new(max_sectors),
            dirty_sectors: HashSet::new(),
            max_cached: max_sectors,
        })
    }
    
    pub fn get(&mut self, lba: u64) -> Option<&[u8]> {
        self.cache.get(&lba).map(|v| v.as_slice())
    }
    
    pub fn put(&mut self, lba: u64, data: Vec<u8>, dirty: bool) {
        if dirty {
            self.dirty_sectors.insert(lba);
        }
        self.cache.put(lba, data);
    }
    
    pub async fn flush_dirty(&mut self, backend: &mut dyn DiskBackend) -> Result<()> {
        let tx = self.db.transaction(&["sectors"], TransactionMode::Readwrite);
        let store = tx.object_store("sectors")?;
        
        for lba in self.dirty_sectors.drain() {
            if let Some(data) = self.cache.get(&lba) {
                // Write to backend
                backend.write_sectors(lba, data)?;
                
                // Optionally persist in IndexedDB for faster loads
                store.put(&SectorRecord { lba, data: data.clone() })?;
            }
        }
        
        tx.done().await?;
        backend.flush()?;
        Ok(())
    }
}
```

---

### Snapshot/Restore (Save States)

Storage snapshots must be **durable** and **deterministic**:

#### What must be captured

- **IDE / AHCI / NVMe controller state**
  - full register sets (MMIO + PCI config where relevant)
  - command list / queue base pointers, head/tail indices
  - in-flight commands (slot/cid, PRDT/SG lists, transfer progress)
  - pending interrupts
- **Disk layer state**
  - backend identity (OPFS file path / IDB database name + object store)
  - optional read cache contents (hot sectors)
  - write-back cache contents (if any) and dirty tracking
  - flush-in-progress status

#### Snapshot protocol requirements

1. **Force flush**: before accepting a snapshot, the I/O worker must flush any dirty write-back cache to the backing store.
2. **Capture disk metadata**: record enough information to reopen OPFS/IDB handles on restore.
3. **Versioned encoding**: snapshots must include a version header and be forward-compatible (unknown fields skipped).

#### Restore semantics

- On restore, the I/O worker must **reopen OPFS/IDB handles** and then rehydrate any in-memory caches.
- If a snapshot is taken mid-command, the controller must resume from the captured in-flight command state (or abort the command in a guest-visible way if unsupported).

---

### Aero sparse disk format (`AEROSPAR`)

Aero’s current sparse disk format (`AEROSPAR`) is implemented in the canonical Rust disk stack as
`aero_storage::AeroSparseDisk`:

- Implementation: [`crates/aero-storage/src/sparse.rs`](../../crates/aero-storage/src/sparse.rs)
- Magic: ASCII `AEROSPAR` (8 bytes), version **1**, header size **64 bytes**

High-level layout:

- Header (64 bytes)
- Allocation table: `table_entries` little-endian `u64` values
  - each entry stores the **physical byte offset** of the corresponding data block, or `0` if the
    logical block is unallocated (reads return zeros)
- Data region: fixed-size blocks appended as they are allocated (`data_offset` is aligned to
  `block_size_bytes`)

On-disk header fields (offsets in bytes; all little-endian):

```text
0x00  8   magic = "AEROSPAR"
0x08  4   version = 1
0x0C  4   header_size = 64
0x10  4   block_size_bytes
0x14  4   reserved
0x18  8   disk_size_bytes
0x20  8   table_offset = 64
0x28  8   table_entries = ceil(disk_size_bytes / block_size_bytes)
0x30  8   data_offset = align_up(table_end, block_size_bytes)
0x38  8   allocated_blocks (physical block slots in the data region; high-water mark)
```

Notes:

- `block_size_bytes` must be a power-of-two multiple of 512 (and is capped at 64 MiB); **1 MiB**
  is a common default.
- `AeroSparseDisk` supports best-effort deallocation (`discard_range`) for fully covered blocks.

---

### NVMe Emulation (Optional Performance Path)

NVMe provides a simpler, higher-throughput datapath than AHCI (doorbells + DMA queues).
The reference implementation lives in `crates/aero-devices-nvme/` and is designed to plug
into Aero’s `aero_storage::VirtualDisk` abstraction and `memory::MemoryBus` DMA interface.

NVMe LBAs are currently fixed at **512 bytes** in this device model. NVMe constructors validate
that the attached `VirtualDisk` has a capacity that is a multiple of 512.

#### Implemented (MVP)

- **PCI device model**
  - BAR0 register set: `CAP/VS/CC/CSTS/AQA/ASQ/ACQ` + doorbells.
  - Admin submission/completion queues.
  - I/O submission/completion queues created via admin commands.
- **Commands**
  - Admin: `IDENTIFY`, `CREATE/DELETE IO CQ`, `CREATE/DELETE IO SQ`, `GET/SET FEATURES`.
  - I/O: `READ`, `WRITE`, `FLUSH`, `WRITE ZEROES`, `DATASET MANAGEMENT (DSM deallocate)`.
- **DMA**
  - PRP1/PRP2 + PRP list support for multi-page transfers.
  - Limited SGL support for READ/WRITE:
    - Data Block descriptors (address + length).
    - Segment / Last Segment chaining (bounded).
- **Interrupts**
  - Legacy INTx signalling (sufficient to boot most guests).
  - Single-vector MSI is supported by the NVMe PCI wrapper when enabled by the guest and wired up
    by the platform (`aero_platform::interrupts::msi::MsiTrigger`). When MSI is active, legacy
    INTx is suppressed.
  - MSI-X is supported (currently a single vector) with a BAR0-backed table/PBA region:
    - Table at `BAR0 + 0x3000`
    - PBA at `BAR0 + 0x3010` (8-byte aligned)
    - When MSI-X is enabled, NVMe completions trigger MSI-X deliveries (vector 0) instead of
      MSI/INTx.
  - Tests: `crates/aero-devices-nvme/tests/interrupts.rs` (MSI vs MSI-X preference + INTx
    suppression).

#### Windows 7 Compatibility (Driver Requirements)

Windows 7 does **not** ship with an in-box NVMe driver. For Windows 7 guests, NVMe must be
treated as **experimental** unless the guest is provisioned with an NVMe driver.

Options (do not redistribute third-party binaries in-repo):

1. **Microsoft hotfixes (commonly referenced):**
   - KB2990941 (adds NVMe support)
   - KB3087873 (NVMe-related fixes)
2. **Vendor/third-party NVMe drivers** (e.g. SSD vendor drivers) installed inside the guest.

For maximum out-of-the-box compatibility, keep AHCI as the default controller and enable NVMe
only for performance experimentation.

---

### Virtio-blk (Paravirtualized)

For maximum performance, we provide virtio-blk drivers:

> For the exact Windows 7 driver ↔ Aero device-model interoperability contract (PCI transport, virtqueue rules, and virtio-blk requirements), see:  
> [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md)  
>
> For the split-ring virtqueue implementation algorithms used by Windows 7 KMDF virtio drivers, see:  
> [`../specs/windows7-virtio-driver-contract.md`](../specs/windows7-virtio-driver-contract.md)  
>
> Windows 7 x64 enforces kernel-mode driver signatures. For test-signed virtio drivers (and the required `boot.wim`/`install.wim` + BCD servicing steps), see [16 - Windows 7 Install Media Servicing (WinPE/Setup) for Test-Signed Virtio Drivers](windows-drivers.md).

```rust
pub struct VirtioBlkDevice {
    // Virtio common config
    device_features: u64,
    driver_features: u64,
    
    // Device-specific config
    capacity: u64,
    size_max: u32,
    seg_max: u32,
    blk_size: u32,
    
    // Virtqueues
    request_vq: Virtqueue,
    
    // Backend
    disk: Box<dyn aero_storage::VirtualDisk>,
}

#[repr(C)]
pub struct VirtioBlkRequest {
    req_type: u32,   // VIRTIO_BLK_T_IN, OUT, FLUSH, etc.
    reserved: u32,
    sector: u64,
}

impl VirtioBlkDevice {
    pub fn process_queue(&mut self, memory: &mut MemoryBus) {
        while let Some(desc_chain) = self.request_vq.pop_available(memory) {
            // First descriptor: request header
            let header_addr = desc_chain[0].addr;
            let request: VirtioBlkRequest = memory.read_struct(header_addr);
            
            // Middle descriptors: data buffers
            let mut data_bufs: Vec<(u64, u32)> = desc_chain[1..desc_chain.len()-1]
                .iter()
                .map(|d| (d.addr, d.len))
                .collect();
            
            // Last descriptor: status byte
            let status_addr = desc_chain.last().unwrap().addr;
            
            let status = match request.req_type {
                VIRTIO_BLK_T_IN => self.do_read(request.sector, &data_bufs, memory),
                VIRTIO_BLK_T_OUT => self.do_write(request.sector, &data_bufs, memory),
                VIRTIO_BLK_T_FLUSH => self.do_flush(),
                _ => VIRTIO_BLK_S_UNSUPP,
            };
            
            // Write status
            memory.write_u8(status_addr, status);
            
            // Push completion
            self.request_vq.push_used(desc_chain.head_id, 1, memory);
        }
        
        // Signal guest
        if self.request_vq.should_notify() {
            self.raise_irq();
        }
    }
}
```

---

### CD-ROM/DVD Emulation

For Windows 7 installation:

> Note: If Aero uses custom/paravirtual storage devices, Windows Setup may require a
> "Load Driver" step. CI can optionally produce a small FAT32 driver disk image for
> this scenario; see [Driver Install Media (FAT Image)](windows-drivers.md).

```rust
pub struct CdromDrive {
    // ATAPI interface
    atapi_state: AtapiState,
    
    // Disc image
    image: Option<IsoImage>,
    
    // State
    tray_open: bool,
    media_changed: bool,
}

impl CdromDrive {
    pub fn execute_packet_command(&mut self, packet: &[u8; 12], memory: &mut MemoryBus) -> AtapiResult {
        let opcode = packet[0];
        
        match opcode {
            ATAPI_READ_10 => {
                let lba = u32::from_be_bytes([packet[2], packet[3], packet[4], packet[5]]);
                let length = u16::from_be_bytes([packet[7], packet[8]]) as u32;
                self.read_sectors(lba, length, memory)
            }
            ATAPI_READ_12 => {
                let lba = u32::from_be_bytes([packet[2], packet[3], packet[4], packet[5]]);
                let length = u32::from_be_bytes([packet[6], packet[7], packet[8], packet[9]]);
                self.read_sectors(lba, length, memory)
            }
            ATAPI_READ_CAPACITY => {
                self.get_capacity(memory)
            }
            ATAPI_READ_TOC => {
                self.read_toc(packet, memory)
            }
            ATAPI_START_STOP_UNIT => {
                let loej = (packet[4] >> 1) & 1;
                let start = packet[4] & 1;
                self.start_stop(loej != 0, start != 0)
            }
            ATAPI_TEST_UNIT_READY => {
                self.test_unit_ready()
            }
            ATAPI_INQUIRY => {
                self.inquiry(memory)
            }
            ATAPI_MODE_SENSE => {
                self.mode_sense(packet, memory)
            }
            _ => {
                AtapiResult::Error(ATAPI_SENSE_ILLEGAL_REQUEST)
            }
        }
    }
}
```

---

### Disk Image Management

#### Read-only base images, ISO attachments, and writeback overlays

In production, Aero often combines **immutable base media** with a **local writable overlay**:

- **Base images** (golden OS installs, demo images, CDN-hosted images) should be treated as
  **read-only**. This is especially important for:
  - chunked delivery ([18 - Chunked Disk Image Format](../specs/chunked-disk-image-format.md)), which is
    designed for read-only content, and
  - shared base images that may be mounted by many sessions/users.
- **ISO attachments** (Windows install media, driver ISOs) are also read-only and should be exposed
  as such to the guest.

To make this intent explicit (and to avoid accidental mutation), prefer wrapping base media in a
read-only adapter at the disk/backing-store layer (e.g. `aero_storage::ReadOnlyDisk` /
`aero_storage::ReadOnlyBackend` around the
canonical `aero_storage::{VirtualDisk, StorageBackend}` traits; see
[`20-storage-trait-consolidation.md`](storage.md)).

Writes then go to a per-session/per-user overlay such as `aero_storage::AeroCowDisk` or a sparse
writeback file in OPFS.

#### Image Download and Streaming

Remote disk images can be streamed on-demand using HTTP `Range` requests while opportunistically caching fetched data into a local sparse file (OPFS).
To maximize cache hit-rate (especially when a CDN sits in front of the disk server), the client should:

- **Align reads to a fixed `CHUNK_SIZE`** (default: **1 MiB**).
- **Reuse the same chunk boundaries** for all requests (always fetch whole chunks like `bytes=N..N+CHUNK_SIZE-1`), rather than issuing variable-sized ranges.

Protocol-level requirements for authenticated disk streaming (HTTP `Range`, auth styles, CORS/COEP/CORP) are specified in [16 - Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](storage.md).
Operational details for the backend **disk image streaming service** (deployment, troubleshooting) are documented in [backend/disk-image-streaming-service.md](storage.md).

The example below uses HTTP `Range` requests for random-access reads. For a CDN-friendly alternative that avoids `Range` (and therefore avoids CORS preflight on cross-origin fetches), see [18 - Chunked Disk Image Format](../specs/chunked-disk-image-format.md).
For how disk/ISO images are uploaded/imported into a hosted service and kept private over time (including lease scopes and writeback options), see [Disk Image Lifecycle and Access Control](storage.md).

Useful tooling in this repo:

- Correctness + CORS conformance checks: [`tools/disk-streaming-conformance/`](../decisions/README.md)
- Range throughput + CDN cache probing (`X-Cache`): [`tools/range-harness/`](../decisions/README.md)
- Native reference implementations (host-side, non-wasm32):
  - `aero_storage::StreamingDisk` (HTTP `Range` + persistent cache)
  - `aero_storage::ChunkedStreamingDisk` (chunked manifest + per-chunk `GET`, no `Range`)
- Chunked disk publisher (no-`Range` delivery): [`tools/image-chunker/`](../decisions/README.md)
  - `publish --format <raw|qcow2|vhd|aerosparse|auto>` (alias: `aerospar`): publish the **logical disk byte stream** for common container formats, not just raw `.img` files.
  - `verify`: validate uploaded `manifest.json` + `chunks/*.bin` end-to-end (schema, existence, sizes, optional per-chunk sha256). Supports S3-backed verification (`--bucket`), direct HTTP verification (`--manifest-url`), and local verification (`--manifest-file`).

```rust
pub struct DiskAccessLease {
    // Prefer a same-origin URL such as `/disk/<lease_id>` to avoid CORS preflight.
    remote_url: String,

    // Short-lived auth for private images. Public images may omit this, and signed-URL schemes
    // may embed auth material directly in `remote_url` instead.
    bearer_token: Option<String>,
    expires_at: Option<SystemTime>,
}

pub struct StreamingDisk {
    // Remote image access (may be unauthenticated for public images)
    lease: DiskAccessLease,
    total_size: u64,
    
    // Local cache
    // (e.g. an OPFS-backed `AEROSPAR` sparse disk file)
    local_cache: AeroSparseDisk,
    
    // Download state
    downloaded_ranges: RangeSet,
    pending_fetches: HashMap<u64, oneshot::Sender<Vec<u8>>>,
}

impl StreamingDisk {
    pub async fn read_sectors(&mut self, lba: u64, buffer: &mut [u8]) -> Result<()> {
        let byte_offset = lba * 512;
        let byte_end = byte_offset + buffer.len() as u64;
        
        // Check if we have this range cached
        if self.downloaded_ranges.contains_range(byte_offset, byte_end) {
            // Read from local cache
            return self.local_cache.read_at(byte_offset, buffer);
        }
        
        // Need to fetch from remote
        let chunk_start = (byte_offset / CHUNK_SIZE) * CHUNK_SIZE;
        let chunk_end = ((byte_end + CHUNK_SIZE - 1) / CHUNK_SIZE) * CHUNK_SIZE;
         
        // Fetch chunk
        let data = self.fetch_range(chunk_start, chunk_end).await?;
        
        // Store in local cache
        self.local_cache.write_at(chunk_start, &data)?;
        self.downloaded_ranges.insert(chunk_start, chunk_end);
        
        // Return requested portion
        let offset_in_chunk = (byte_offset - chunk_start) as usize;
        buffer.copy_from_slice(&data[offset_in_chunk..offset_in_chunk + buffer.len()]);
        
        Ok(())
    }
    
    async fn fetch_range(&mut self, start: u64, end: u64) -> Result<Vec<u8>> {
        // Proactively refresh shortly before expiry so long-running sessions don't stall on auth.
        if let Some(expires_at) = self.lease.expires_at {
            if SystemTime::now() + LEASE_REFRESH_SKEW >= expires_at {
                self.refresh_lease().await?;
            }
        }

        // NOTE: Cross-origin requests with `Range` (and `Authorization` when present) trigger a
        // CORS preflight; prefer a same-origin `/disk/...` endpoint to avoid preflight entirely.
        let mut headers = vec![
            ("Range".to_string(), format!("bytes={}-{}", start, end - 1)),
        ];
        if let Some(token) = &self.lease.bearer_token {
            headers.push(("Authorization".to_string(), format!("Bearer {}", token)));
        }

        let mut response = fetch(&self.lease.remote_url, FetchOptions { headers }).await?;
        
        // If the lease expires (or is revoked) mid-run, refresh the lease and retry once.
        if response.status() == 401 || response.status() == 403 {
            self.refresh_lease().await?;

            let mut headers = vec![
                ("Range".to_string(), format!("bytes={}-{}", start, end - 1)),
            ];
            if let Some(token) = &self.lease.bearer_token {
                headers.push(("Authorization".to_string(), format!("Bearer {}", token)));
            }

            response = fetch(&self.lease.remote_url, FetchOptions { headers }).await?;
        }

        Ok(response.bytes().await?)
    }

    async fn refresh_lease(&mut self) -> Result<()> {
        // Fetch a new short-lived disk access lease (remote_url + optional auth) from the backend.
        // The previous token/URL should be treated as secret and discarded.
        self.lease = request_new_lease_from_backend().await?;
        Ok(())
    }
}
```

##### Production backend requirements (Range/CORS/no-transform)

`StreamingDisk.lease.remote_url` is expected to point at a production-grade object delivery path (typically **CDN → S3/object store**) that supports efficient random access via `HTTP Range`.

For full deployment guidance (S3 CORS, CloudFront caching policies, signed cookies/URLs, versioned keys), see: [16 - Remote Disk Image Delivery (Object Store + CDN + HTTP Range)](storage.md)

`StreamingDisk` reads remote disk images by issuing `Range` requests and writing returned bytes directly into a local sparse cache. The remote server **must** behave like a compliant byte-range server; returning a full `200 OK` body (ignoring `Range`) will corrupt the virtual disk because offsets no longer match.

Minimum contract:

- `HEAD` must return `Content-Length` (total size) plus `ETag`/`Last-Modified` for versioning.
- The server **must** support HTTP `Range` requests for the `bytes` unit (`Accept-Ranges: bytes` is recommended).
- For a satisfiable range request, respond with:
  - `206 Partial Content`
  - `Content-Range: bytes <start>-<end>/<total>` (where `<end>` is inclusive)
- For an unsatisfiable range request (past EOF), respond with:
  - `416 Range Not Satisfiable`
  - `Content-Range: bytes */<total>`
- The server should either:
  - implement open-ended ranges (`bytes=<start>-`) and suffix ranges (`bytes=-<suffix-length>`), or
  - explicitly reject them (do not silently ignore the header)
- Multi-range requests (e.g. `Range: bytes=0-0,100-199`) are not used by `StreamingDisk` and are not required for production deployments.
  - Reject multi-range requests explicitly (e.g. `416` or `400`) rather than silently ignoring the header.

Critical implementation constraints:

- Do **not** apply compression or transformations to the disk image response. The client interprets offsets in the raw on-disk byte stream.
  - Recommended: include `Cache-Control: no-transform`
  - Ensure `Content-Encoding` is absent or `identity`
- Cross-origin access (CORS):
  - `Range` is not a CORS-safelisted request header, so browsers will send an `OPTIONS` preflight.
  - The server must allow the request headers used by the client (at minimum `Range`, and `Authorization` if using bearer tokens).
  - The server must expose the response headers needed by the client (see the spec doc for the complete list; typically `Accept-Ranges`, `Content-Range`, `Content-Length`, `ETag`).
- If Aero is deployed with COOP/COEP to enable `SharedArrayBuffer` (`crossOriginIsolated`), disk image resources must be CORS-enabled (`Access-Control-Allow-Origin`) or served with a compatible `Cross-Origin-Resource-Policy` header; otherwise the browser will block the fetch.
  - See [16 - Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](storage.md) for the required CORS/COEP/CORP headers when using authenticated and/or cross-origin streaming.

Concrete examples:

```bash
# Request the first byte (expect 206)
curl -i -H 'Range: bytes=0-0' https://example.com/windows7.img
# HTTP/1.1 206 Partial Content
# Content-Range: bytes 0-0/<total>

# Unsatisfiable range (expect 416)
curl -i -H 'Range: bytes=999999999999-999999999999' https://example.com/windows7.img
# HTTP/1.1 416 Range Not Satisfiable
# Content-Range: bytes */<total>

# Suffix range: last 512 bytes (expect 206)
curl -i -H 'Range: bytes=-512' https://example.com/windows7.img
# HTTP/1.1 206 Partial Content
# Content-Range: bytes <total-512>-<total-1>/<total>
```
#### Authenticated / Private Remote Images

The `StreamingDisk` design supports both **public** and **private** remote disk images.

- **Public images**: `remote_url` can point directly at a CDN/object store URL and `bearer_token` can be omitted.
- **Private images**: access is controlled per-user/per-session and requires a short-lived credential (bearer token *or* a signed URL).

For private images, treat the following as **secrets**:

- `remote_url` (for private images it may embed auth via a signed query parameter, or it may be an unguessable lease URL)
- `bearer_token` / session cookies / any auth material (if used)

Do **not** persist these secrets to OPFS/IndexedDB/localStorage (or logs) “for convenience”. Persist stable identifiers instead (e.g., `image_id`, `snapshot_id`) and reacquire credentials each time a VM session starts.

##### Disk access lease acquisition

Before the emulator sets `StreamingDisk.lease.remote_url`, it should obtain a **disk access lease** from a trusted backend (after user authentication/authorization). The lease is a short-lived blob containing the information needed to stream ranges:

- where to fetch (`remote_url`, preferably same-origin like `/disk/<lease_id>`)
- how to authenticate (e.g., `bearer_token`, or a signed URL embedded into `remote_url`)
- when it expires (`expires_at`)

This keeps long-lived credentials out of the browser and enables fine-grained revocation.

##### Token refresh strategy

Disk streaming is continuous during boot and can run for hours, so the client must handle credential expiry:

- **Proactive refresh**: refresh the lease shortly before `expires_at` (e.g., 30-60s skew) to avoid stalling on the first expired request.
- **Reactive refresh**: if any `fetch_range` request returns `401`/`403`, treat the lease as expired/revoked, request a new lease, then retry the same range once. If it still fails, surface a fatal disk I/O error to the VM session.

If you prefer the **signed URL** style instead of an `Authorization` header, the refresh flow is the same, but it updates `remote_url` rather than `bearer_token`.

##### CORS/COEP constraints for Range streaming

- `Range` header → **CORS preflight on cross-origin** requests.
- If cross-origin fetches are unavoidable, configure the disk endpoint to cache preflights via
  `Access-Control-Max-Age` (see the spec doc for recommended values and caveats).
- If using bearer tokens via `Authorization`, cross-origin fetches also require allowing the
  `Authorization` request header in CORS preflight.
- Prefer a **same-origin** disk streaming endpoint (e.g., `/disk/...`) to avoid preflight and simplify COEP/cross-origin isolation.
- If cross-origin is unavoidable, the disk server must return the CORS/COEP/CORP headers defined in [16 - Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](storage.md).

---

### Performance Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Sequential Read | ≥ 100 MB/s | Large file read |
| Sequential Write | ≥ 50 MB/s | Large file write |
| Random Read IOPS | ≥ 10,000 | 4KB random reads |
| Random Write IOPS | ≥ 5,000 | 4KB random writes |
| Boot Time Impact | < 10s | Additional boot delay |

---

### Next Steps

- See [Audio Subsystem](audio.md) for sound emulation
- See [Browser APIs](web-host.md) for OPFS details
- See [16 - Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](storage.md) for authenticated Range streaming requirements
- See [Task Breakdown](../history/sprint-era-record.md) for storage tasks

## Canonical Storage Topology (Windows 7 Boot/Install)

This document defines the **canonical, deterministic storage device topology** used for Windows 7
boot and installation.

The goal is that **controller emulation**, **platform PCI wiring**, and **browser storage
backends** all agree on:

- which controllers exist,
- where they live on the PCI bus (**BDF**),
- how media is attached (AHCI **port** / IDE **channel+drive**),
- how interrupts are routed (PCI **INTx → GSI** under Aero’s `PciIntxRouterConfig`, plus legacy
  **IDE channel IRQ14/IRQ15** for ATA/ATAPI).

If you change this topology (BDFs, attachment points, or INTx routing), you must update this doc
and the corresponding tests:

- `crates/devices/tests/win7_storage_topology.rs` (PCI profile constants + INTx routing)
- `crates/aero-pc-platform/tests/pc_platform_win7_storage.rs` (platform integration wiring)
- `crates/aero-pc-platform/tests/windows7_storage_topology.rs` (end-to-end AHCI + ATAPI read tests)
- `crates/aero-machine/tests/machine_win7_storage_topology.rs` (canonical `Machine` wiring: BDFs + PCI Interrupt Line)
- `crates/aero-machine/tests/win7_storage_topology.rs` (guard: canonical storage BDFs + PCI Interrupt Line/Pin + IDE legacy BAR ranges + key BAR definitions + optional NVMe BDF/PCI IDs/class/Interrupt Line/BAR0 invariants when enabled)
- `crates/aero-machine/tests/machine_win7_storage_helper.rs` (helper preset wiring)
- `crates/aero-machine/tests/machine_win7_storage.rs` (helper constructor: PCI BDF presence + multifunction ISA bridge)
- `crates/aero-machine/tests/machine_disk_overlays_snapshot.rs` (snapshot `DISKS` disk_id mapping guard test)
- `crates/aero-machine/tests/machine_storage_snapshot_roundtrip.rs` (snapshot/restore: controller state + backend reattach contract)

---

### Summary (normative)

#### Controllers present (canonical Win7 boot/install topology)

| Purpose | Controller | Canonical PCI BDF | Attachment |
|---|---|---:|---|
| Primary HDD (installed OS disk) | AHCI (Intel ICH9) | `aero_devices::pci::profile::SATA_AHCI_ICH9.bdf` (`00:02.0`) | **SATA port 0** (one disk) |
| Install media (boot ISO) | IDE (Intel PIIX3) | `aero_devices::pci::profile::IDE_PIIX3.bdf` (`00:01.1`) | **Secondary channel, master drive** (ATAPI CD-ROM) |

#### Optional controllers (policy)

| Controller | Canonical PCI BDF | Win7 default policy |
|---|---:|---|
| NVMe (QEMU NVMe) | `aero_devices::pci::profile::NVME_CONTROLLER.bdf` (`00:03.0`) | **Off by default** (Win7 lacks inbox NVMe support) |

Rationale: Windows 7 requires hotfixes and/or vendor drivers for NVMe (e.g. KB2990941). For
compatibility-first defaults, keep NVMe disabled unless explicitly opted into by a config/feature.

Implementation note (Rust): `aero_machine::Machine::new_with_win7_storage(...)` /
`aero_machine::MachineConfig::win7_storage(...)` / `aero_machine::MachineConfig::win7_storage_defaults(...)`
are convenience helpers that enable this controller
set at the canonical BDFs for integration tests and bring-up.

These helpers keep non-storage devices conservative by default (for example, E1000 is disabled
unless explicitly enabled by the caller) to reduce drift and keep Win7 install/boot behavior
deterministic.

Lower-level platform-only tests may use `aero_pc_platform::PcPlatform::new_with_win7_storage(...)`
instead.

`aero_pc_platform::PcPlatform::new_with_windows7_storage_topology(...)` additionally attaches an
AHCI HDD (port 0) and an IDE/ATAPI CD-ROM (secondary master) so tests can validate real I/O.

`aero_machine::MachineConfig::win7_storage(...)` / `aero_machine::MachineConfig::win7_storage_defaults(...)`
(or `aero_machine::Machine::new_with_win7_storage(...)`)
enables the same canonical controller set in the full-system `Machine` integration layer.

---

### Boot flows (normative)

#### BIOS boot drive numbering (DL) (normative)

Aero’s legacy BIOS follows PC-compatible drive numbering conventions when transferring control to a
boot sector / El Torito boot image:

| Medium | BIOS drive number | Notes |
|---|---:|---|
| First fixed disk (HDD0) | `DL=0x80` | Boot from the OS disk after install. |
| First ATAPI CD-ROM (CDROM0) | `DL=0xE0` | Boot from install/recovery ISO via El Torito. |

In the canonical machine topology, both an AHCI disk (HDD0) and an IDE/ATAPI CD-ROM (CD0) may be
attached simultaneously.

For BIOS INT 13h, sector units differ by drive number:

- HDD (`DL=0x80..=0xDF`): 512-byte sectors.
- CD-ROM (`DL=0xE0..=0xEF`): 2048-byte logical blocks via INT 13h Extensions (EDD).

Note: In the canonical machine topology, BIOS INT 13h can service **both** HDD0 (`DL=0x80`) and the
install-media CD-ROM (`DL=0xE0`) when both backends are provided (primary disk + install ISO). HDD
drive presence is derived from the BIOS Data Area fixed-disk count (`0x40:0x75`); when booting from
CD, the integration layer ensures this count still advertises HDD0 so guests can access it during
setup/boot.

#### Windows 7 install / recovery flow

1. Select **CD0** as the BIOS boot drive (**`DL=0xE0`**) *before* reset. In `aero_machine`, this is
    `Machine::set_boot_drive(0xE0)` followed by `Machine::reset()`. BIOS transfers control to the CD
    boot image with that CD drive number in `DL`.
    - Optional convenience: hosts may instead enable the firmware “CD-first when present” policy so
      firmware attempts a CD boot when install media is attached and otherwise falls back to the
      configured HDD boot drive (useful for “boot ISO once, then boot HDD after eject”):
      - Rust: `machine.set_cd_boot_drive(0xE0); machine.set_boot_from_cd_if_present(true);` (keep
        `boot_drive=0x80` as the fallback), then `machine.reset()`.
      - Config: `firmware::bios::BiosConfig::cd_boot_drive = 0xE0` +
        `firmware::bios::BiosConfig::boot_from_cd_if_present = true`.
      - Reporting: with this policy enabled, the configured boot drive/device remains the HDD
        fallback (e.g. `boot_drive=0x80` / `BootDevice::Hdd`) even when the current boot actually
        came from CD. Use `Machine::active_boot_device()` (or `Bios::booted_from_cdrom()`) to query
        what firmware actually booted from.
      - Rust convenience: `Machine::configure_win7_install_boot(iso)` (or
        `Machine::new_with_win7_install(...)`) enables this policy, attaches the ISO to the canonical
        ATAPI slot, and resets.
2. The boot ISO must be attached and presented as an **ATAPI CD-ROM** on **PIIX3 IDE secondary
   master** (`disk_id=1`) before BIOS POST/boot:
   - Rust: `Machine::attach_install_media_iso_bytes(...)` (in-memory ISO) or
     `Machine::attach_install_media_iso(...)` (file/stream-backed ISO).
   - Browser/wasm: `machine.attach_install_media_iso_bytes(...)` (copies bytes into WASM memory; OK
     for small ISOs) or `await machine.attach_install_media_iso_opfs(path)` (preferred for large
     ISOs; OPFS-backed, worker-only).
3. BIOS performs an **El Torito no-emulation** boot from that CD drive (see
   [`platform-and-firmware.md`](platform-and-firmware.md) for the detailed El Torito + INT 13h
   expectations).
4. If the CD is absent or unbootable (no ISO, empty tray, invalid boot catalog, etc.), the boot
   attempt will fail; the host should fall back by selecting HDD0 as the boot drive (**`DL=0x80`**)
   and resetting (i.e. configure `firmware::bios::BiosConfig::boot_drive = 0x80`, set
   `aero_machine::MachineConfig::boot_drive = 0x80` at construction time, or in `aero_machine` call
   `Machine::set_boot_drive(0x80)` and `Machine::reset()`).
   - Note: Aero’s BIOS boot selection is still primarily driven by an explicit `boot_drive` (`DL`),
     but it also supports an optional “CD-first when present” policy flag for host convenience.
5. Windows Setup enumerates the **AHCI disk** on **ICH9 AHCI port 0** and installs Windows onto it.

Implementation note (Rust): in `aero_machine`, the initial BIOS boot drive can be set up-front via
`MachineConfig::boot_drive` (for example `MachineConfig::win7_install_defaults(...)` sets
`boot_drive=0xE0` to boot directly from CD0, while the default is `boot_drive=0x80` for HDD boot).
You can also switch at runtime via `Machine::set_boot_drive(0xE0|0x80)` and then call
`Machine::reset()` to re-run BIOS POST with the new `DL` value.

Browser/wasm note: `crates/aero-wasm` exports `Machine` to JS and also exposes
`machine.set_boot_drive(0xE0|0x80)` alongside `machine.reset()`. The same rule applies: set the boot
drive number and then reset to re-run BIOS POST with the new `DL` value.

When available in the active WASM build, the JS-facing `Machine` wrapper also provides
introspection helpers for debugging/automation:

- `machine.boot_drive()` – configured BIOS boot drive (`DL`) used for firmware POST/boot.
- `machine.boot_from_cd_if_present()` – whether the firmware "CD-first when present" policy is enabled.
- `machine.cd_boot_drive()` – CD-ROM boot drive number used when the CD-first policy is enabled.
- `machine.active_boot_device()` – what firmware actually booted from in the current boot session (CD vs HDD).

In the browser runtime, the canonical `vmRuntime="machine"` mode runs the `Machine` inside a worker,
so hosts typically do not have direct access to a `machine` handle on the main thread. For stable
automation-friendly introspection, the runtime also exposes:

- `window.aero.debug.getMachineCpuActiveBootDevice()` – active boot source (CD vs HDD), or `null` if unknown.
- `window.aero.debug.getMachineCpuBootConfig()` – `{ bootDrive, cdBootDrive, bootFromCdIfPresent }`, or `null` if unknown.

Note: both can return `null` during transitions (e.g. right after a reset, snapshot restore, or boot
disk change) before the CPU worker re-reports the updated state.

#### Normal boot (after installation)

1. Select HDD0 as the BIOS boot drive (**`DL=0x80`**) before reset. BIOS enters the boot sector with
   **`DL=0x80`**.
2. The IDE CD-ROM may remain attached (useful for tooling/driver ISOs), but is not required.

---

### Media attachment mapping (normative)

#### AHCI HDD mapping

- The **primary VM disk image** (the installed OS disk) is attached to:
  - **Controller:** ICH9 AHCI (`SATA_AHCI_ICH9`)
  - **Port:** `0`
  - **Snapshot `disk_id`:** `0`

Notes:
- The AHCI device model can support multiple ports, but the **canonical Win7 topology** attaches
  exactly one disk at **port 0** (and typically instantiates the controller with a single
  implemented port) for determinism.
- Do not “move” the OS disk to another port without updating this document and any frontend/storage
  assumptions (e.g. “disk0 == AHCI port0”).

#### IDE / ATAPI CD-ROM mapping

- The **Windows install ISO** (or other optical media) is attached to:
  - **Controller:** PIIX3 IDE (`IDE_PIIX3`)
  - **Channel:** secondary
  - **Drive:** master
  - **Snapshot `disk_id`:** `1`

This matches the explicit attachment API in the IDE model:
`aero_devices_storage::pci_ide::IdeController::attach_secondary_master_atapi(...)`.

Note: `IDE_PIIX3` is PCI function `00:01.1`. For OS enumeration to reliably discover it, the
platform should also expose the PIIX3 ISA bridge function (`aero_devices::pci::profile::ISA_PIIX3`
at `00:01.0`) with the multi-function bit set in its header type (see
`platform-and-firmware.md`).

---

### Snapshot DISKS mapping (DiskOverlayRefs) (normative)

Snapshots store disk/ISO *references* separately from device/controller state in the `DISKS`
section as `aero_snapshot::DiskOverlayRefs` entries (see [`../specs/snapshot-format.md`](../specs/snapshot-format.md)).
Each entry is a `DiskOverlayRef { disk_id, base_image, overlay_image }` keyed by a stable `disk_id`
(`u32`) that identifies a **logical attachment point** in the machine topology.

Note: `base_image` / `overlay_image` are opaque host identifiers and may be empty strings to
represent "not configured".

Browser/runtime convention (web machine runtime mode, `vmRuntime=machine` / `?vm=machine`):
`base_image` / `overlay_image` are interpreted as **OPFS-relative paths** (e.g.
`aero/disks/win7.img`). Paths are relative to the OPFS root (no leading `/`) and must not contain
`..` segments.

When restoring a snapshot, storage controller device snapshots intentionally restore only
guest-visible controller state and **drop any attached host backends** (disk files, ISO handles,
etc.). The host/runtime must:

1. Read the snapshot `DISKS` section.
2. Re-open the referenced base/overlay images on the host.
3. Re-attach those reopened backends to the canonical attachment points below **before** resuming
   guest execution.

#### Canonical `disk_id` values for Win7

| `disk_id` | Attachment point | Purpose |
|---:|---|---|
| `0` | ICH9 AHCI, **SATA port 0** (`00:02.0`) | Primary HDD (installed OS disk) |
| `1` | PIIX3 IDE, **secondary channel master ATAPI** (`00:01.1`) | Install media ISO (CD-ROM) |
| `2` | PIIX3 IDE, **primary channel master ATA** (`00:01.1`) | Optional IDE primary master ATA disk |

This mapping is implemented as stable constants in the canonical machine integration layer:
`aero_machine::Machine::DISK_ID_*`.

These `disk_id` values are part of the Win7 platform ABI: changing them breaks deterministic
snapshot restore unless all producers/consumers are updated in lockstep.

Guard test: `crates/aero-machine/tests/machine_disk_overlays_snapshot.rs`.

---

### Legacy IDE compatibility expectations (normative)

Even though the IDE controller is a PCI function (`00:01.1`), the canonical topology exposes it in
**legacy compatibility mode** so classic BIOS/boot-loader software and Windows 7’s IDE/ATAPI stack
can talk to it via the fixed ISA-compatible I/O port map.

#### Legacy port map (compat mode)

| Channel | Command block | Control block (alt status/devctl) | IRQ |
|---------|---------------|------------------------------------|-----|
| Primary | `0x1F0..=0x1F7` | `0x3F6..=0x3F7` | IRQ14 |
| Secondary | `0x170..=0x177` | `0x376..=0x377` | IRQ15 |

#### PCI BAR expectations (PIIX3 IDE)

This matches QEMU `piix3-ide`: programming interface **`0x80`** (legacy ATA + bus-master
DMA, channels not programmable) and **only BAR4** implemented.

| BAR | Meaning | Expected base |
|-----|---------|---------------|
| BAR0–3 | Unimplemented (read as 0) | — |
| BAR4 | Bus Master IDE (BMIDE), 16 bytes | default `0xC000` (relocatable) |

Command/control blocks are **not** PCI BARs. They stay hardwired at the ISA
compatibility ports in the table above. Advertising BAR0–3 as I/O windows (the
old `prog-if 0x8A` layout) makes Windows 7 `pci.sys` treat the function as
native-capable: it sizes those BARs, leaves them unassigned, and never adds
`0x1F0`/`0x170` + IRQ14/15.

PIIX3 **IDETIM** (PCI config `0x40` primary / `0x42` secondary) bit 15 is
channel decode-enable. Firmware and QEMU/KVM set it; a reset-0 register makes
`intelide.sys` create **no channel PDOs**, so the ATAPI CD-ROM never appears
even when the ISO is attached. Aero sets both channels to `0x8000` at construct,
reset, and snapshot restore (including the guest-visible PCI-bus copy).

Taskfile writes match QEMU: each write updates the visible register and the
previous value becomes the HOB. The old “first write lives in HOB until the
command” scheme made Win7 ataport read back a stale ATAPI signature instead of
the PACKET byte-count limit it had just programmed (INQUIRY is BCL `0x24`).
The PACKET command sets DRQ for the 12-byte CDB but does **not** raise INTRQ.
QEMU `cmd_packet` **assigns** Sector Count = `ATAPI_INT_REASON_CD` (1); it
does not OR into a leftover SET FEATURES transfer-mode count. Subsequent
ATAPI completions preserve bits 7:3 (`nsector & ~7 | reason`), matching
`ide_atapi_cmd_reply_end` / `ide_atapi_cmd_ok`. Packet errors are
READY|ERR with `error = sense_key<<4` and no DSC.

Command-block writes other than the command register are ignored while
BSY or DRQ, matching QEMU `ide_ioport_write`. The data port stays live.

IRQ14/IRQ15 are driven on the same I/O that changes the latch (`set_irq` /
status-read `clear_irq`), not only at batch-boundary `poll_pci_intx_lines`.
Win7 `ataport` ATAPI PIO is a two-IRQ protocol (data-in, then status). The
status-phase edge is raised by the last data-port read inside the same
Tier-0 I/O batch that acked the data-in IRQ. A poll-only pin never goes
1→0→1, so DiscoverLuns INQUIRY times out and no `IDE\CdRom` PDO appears.
IDENTIFY still completed because its ATA path finishes in one ISR.

GET CONFIGURATION (MMC 0x46) is ALLOW_UA and does not require media. The
response is QEMU's feature-0 Profile List (DVD-ROM 0x0010 + CD-ROM 0x0008);
current profile is 0 with no media, CD-ROM for ≤360000 2048-byte sectors,
DVD-ROM otherwise. A non-zero starting feature is ILLEGAL REQUEST.

#### AHCI descriptor entries are byte-granular, not sector-aligned

A physical-region descriptor is a **scatter-gather fragment**, and nothing
requires it to begin or end on a sector boundary. The device must assemble and
scatter *whole sectors* across descriptor boundaries.

Assuming otherwise is not a theoretical concern: Windows 7's `FormatEx` splits a
single 512-byte sector across two descriptors of 100 and 412 bytes when writing
the master file table. A disk backend that required every transfer to be a
multiple of the sector size aborted the first fragment, and the failure surfaced
four layers up as a bare `0x80070057` (`E_INVALIDARG`) from Setup, with no disk
writes at all and an overlay that never grew.

Both directions carry regressions (`write_dma_split_prdt_unaligned_entries_roundtrip`
in `crates/aero-devices-storage/src/ahci.rs`, plus its read twin) — whenever a
scatter-gather assembly assumption is fixed on one path, the same assumption is
almost always present on the other.

#### AHCI MMIO (ABAR) expectation

The ICH9 AHCI controller exposes the ABAR register block via **BAR5** (MMIO), size
`aero_devices::pci::profile::AHCI_ABAR_SIZE`.

The PCI config space register offset for the ABAR itself is
`aero_devices::pci::profile::AHCI_ABAR_CFG_OFFSET`.

---

### Interrupt routing expectations (normative)

#### PCI INTx routing (AHCI / NVMe / IDE PCI config)

Aero’s canonical PCI INTx routing uses:

- `aero_devices::pci::irq_router::PciIntxRouterConfig::default()`
  - `PIRQ[A-D] -> GSI[20,21,22,23]`
- Root bus swizzle:
  - `PIRQ = (INTx + device_number) mod 4`

Since the PCI INTx storage controllers use `INTA#`, their routed GSIs are determined purely by the
PCI **device number**:

| Device | BDF | INTx pin | PIRQ | GSI |
|---|---:|---:|---:|---:|
| PIIX3 IDE | `00:01.1` | INTA | B | **21** |
| ICH9 AHCI | `00:02.0` | INTA | C | **22** |
| NVMe (optional) | `00:03.0` | INTA | D | **23** |

This table is validated by `crates/devices/tests/win7_storage_topology.rs`.
The corresponding legacy PIC lines are 11, 12, and 13.

#### Legacy IDE channel interrupts (ATA/ATAPI)

Even though PIIX3 IDE is a PCI function (`00:01.1`), the **data-plane** interrupts for legacy
IDE/ATAPI are the traditional ISA IRQs:

- **Primary channel:** ISA IRQ **14**
- **Secondary channel:** ISA IRQ **15**

In Aero’s platform interrupt controller (`aero_platform::interrupts::PlatformInterrupts`), ISA IRQs
map to GSIs of the same number by default (except IRQ0 → GSI2), so these correspond to:

- primary IDE: **GSI 14**
- secondary IDE: **GSI 15**

Note: `IDE_PIIX3.build_config_space()` still exposes a PCI `Interrupt Pin` (`INTA#`) and an
`Interrupt Line` consistent with `PciIntxRouterConfig::default()` (currently **11**), but the
canonical IDE/ATAPI completion interrupts that software relies on are IRQ14/IRQ15.

## Disk Images: Local vs Streaming (HTTP Range / Chunked)

### Overview

Aero can use **disk images** in two ways:

1. **Local images**: you provide a file (or one is generated) and Aero stores it in browser storage
   (OPFS preferred; IndexedDB fallback in some environments). Local import flows can accept common
   container formats (e.g. qcow2/VHD) and convert them into Aero’s internal sparse-on-OPFS format
   (`AEROSPAR`) for efficient random access. See: [`16-disk-image-management.md`](storage.md).
2. **Streaming images**: you provide a **URL** to a remote image and Aero reads it lazily, caching only the blocks/chunks it actually touches.
    - **HTTP Range** (single file): fetches with `Range: bytes=...`
    - **Chunked manifest** (many files): fetches `manifest.json` + `chunks/*.bin` with plain `GET` (no `Range` header)

Streaming is essential for very large images (20GB+) because it avoids a full upfront download.

Note: OPFS is the preferred backend for local images. In this repo, the primary OPFS backend is
implemented in Rust/wasm32 in `crates/aero-opfs`.

Aero can fall back to IndexedDB for some host-side storage flows when OPFS sync access handles are
unavailable, but IndexedDB is async-only and does not currently back the synchronous Rust
disk/controller path; see
[`19-indexeddb-storage-story.md`](storage.md) and the canonical trait mapping in
[`20-storage-trait-consolidation.md`](storage.md).

### Legal / Responsible Use (Important)

You must only use disk images that you **own** or are otherwise **licensed to use**.

Do **not** use Aero’s streaming support to access or distribute pirated Windows installers or disk images. If you don’t have explicit rights to the content, don’t point Aero at it.

### Streaming images: server requirements

#### Mode A: HTTP Range (single object)

To stream a remote image, the server **must** support byte-range requests:

- `Accept-Ranges: bytes`
- `Content-Length` on `HEAD`/`GET`
- Correct `206 Partial Content` responses to `Range: bytes=start-end`
- Correct `Content-Range: bytes start-end/total`

Notes:

- Some servers disallow `HEAD`. Aero can fall back to a small `Range: bytes=0-0` probe,
  but that requires a valid `Content-Range` header (and appropriate CORS exposure).

#### CORS headers (browser requirement)

Browsers will block cross-origin reads unless the server is configured for CORS.

For a self-contained local setup (MinIO + optional reverse proxy) to validate Range + CORS behavior, see:
[`infra/local-object-store/README.md`](../decisions/README.md).

Because `Range` is not a CORS-safelisted request header, cross-origin reads will trigger an `OPTIONS`
preflight. At minimum, the server should respond with headers similar to:

*Preflight (`OPTIONS`) response*:

```
Access-Control-Allow-Origin: https://your-aero-origin.example
Access-Control-Allow-Methods: GET, HEAD, OPTIONS
Access-Control-Allow-Headers: Range, If-Range, If-None-Match, If-Modified-Since
```

*Disk bytes (`GET`/`HEAD`) response*:

```
Access-Control-Allow-Origin: https://your-aero-origin.example
Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding
```

Notes:

- `Access-Control-Allow-Origin: *` is acceptable for public, non-credentialed access.
- `Content-Range` is not a “simple” header, so it must be **exposed** if the UI needs to read it.
  - This matters for HTTP Range mode. Chunked mode does not read `Content-Range`.
- `Last-Modified` is a CORS-safelisted response header and is exposed to JS by default (no `Expose-Headers` needed).

### Streaming images: caching behavior

The streaming backend downloads data in fixed-size **blocks** (default: **1 MiB** for HTTP Range mode, or **4 MiB** chunks for the chunked format).

- On a cache miss, Aero fetches the required block/chunk from the remote server (Range `GET` or chunk `GET`).
- Blocks/chunks are stored locally and reused on subsequent runs.
- A cache size limit can be configured; when exceeded, least-recently-used blocks/chunks are evicted.

#### Mode B: Chunked manifest (no `Range`)

Chunked streaming avoids `Range` requests entirely (and therefore can avoid CORS preflight in many deployments):

- You host a `manifest.json` plus fixed-size `chunks/00000000.bin`, `chunks/00000001.bin`, etc.
- The client reads the manifest once, then fetches chunks with plain `GET`.

See the full format spec: [`18-chunked-disk-image-format.md`](../specs/chunked-disk-image-format.md).

To generate a chunked image (manifest + chunks) from a local disk image, use the reference tooling:

- [`tools/image-chunker/`](../decisions/README.md) (`aero-image-chunker publish`)
  - Supports `publish --format` to publish the **logical disk byte stream** for container formats
    (qcow2/VHD/AeroSparse), not just raw `.img` files.
    - Note: images that require an explicit parent (QCOW2 backing files, VHD differencing) should be flattened before chunking.
  - `--format aerosparse` also accepts the legacy alias `aerospar`.
  - Provides `aero-image-chunker verify` to validate a chunked image end-to-end:
    - S3 mode via `--bucket` + `--prefix` / `--manifest-key`
    - direct HTTP via `--manifest-url` (e.g. CDN-hosted images)
    - local verification via `--manifest-file` (no S3 required)
    - verification is fail-fast (stops on the first mismatch); use `--chunk-sample N` for smoke checks
 
### Inspecting streaming performance (telemetry + controls)

The dev UI includes a **Remote disk image (streaming)** panel that can open a remote disk via the runtime disk worker and display live stats:

- total image size
- cached bytes + configured cache limit
- cache hit rate
- bytes downloaded + request counts
- outstanding in-flight fetches

It also provides buttons to **flush metadata**, **clear cache**, and **close** the streaming handle so you can tune block/chunk sizing and cache limits for 20GB+ boot scenarios.

Additional tuning knobs:

- **Credentials mode** (`same-origin` / `include` / `omit`) for cookie-auth / credentialed CORS setups.
- **Cache key override** (`cacheImageId`, `cacheVersion`) so you can pin cache identity separately from the URL
  (useful when URLs include ephemeral auth query params or when you want to force a cache bust by bumping a version).
- **Reset stats**: records a baseline snapshot and shows deltas for cumulative counters without clearing the cache.
- **Settings persistence**: the panel stores its last-used options in `localStorage` (URLs are stored without query/hash).
- **Cache limit**: set a positive MiB value to enable LRU eviction; set `0` to disable eviction (unbounded cache growth).

### Security / UX expectations

Remote image support should be gated behind explicit user action:

- A dedicated “Use remote image” toggle (off by default).
- A URL input field.
- A clear warning that remote images can be untrusted and may leak request metadata to the host.
- A cache/progress indicator (downloaded blocks, cache size, etc.).

## IndexedDB storage story (async vs sync)

See also:

- [`20-storage-trait-consolidation.md`](storage.md) (repo-wide disk/backend trait inventory + canonical trait guidance)

### Context / problem statement

In this repo there are **two different “storage stacks”**:

1. **The Rust disk + controller stack** (boot-critical, used by the Rust AHCI/IDE device models)
   - `crates/aero-storage::{StorageBackend, VirtualDisk}` are **synchronous** traits.
   - `crates/aero-devices-storage` (AHCI/IDE/ATAPI) expects a `Box<dyn VirtualDisk>` and performs
     **synchronous** reads/writes during command processing.
   - In the browser, the intended synchronous backend is **OPFS SyncAccessHandle** via
     `crates/aero-opfs` (`aero_opfs::OpfsByteStorage` / `aero_opfs::OpfsBackend`).

2. **Async browser storage utilities** (UI/disk manager/import/export/benchmarks)
   - `crates/st-idb` is an **async** IndexedDB-backed block store.
   - The web runtime already contains async disk abstractions (`apps/web/src/storage/*`).

This creates an integration gap:

- IndexedDB is fundamentally **async**.
- The Rust controller path is **sync**.
- In a browser Worker, attempting to “pretend” IndexedDB is sync by blocking the thread will
  deadlock (details below).

There is also an easy-to-miss footgun today:

- `crates/aero-opfs` can fall back to `OpfsIndexedDbBackend`, but that backend is **async-only**
  and therefore **cannot** back `aero_storage::VirtualDisk` or the synchronous Rust controller path.

The goal of this doc is to make the IndexedDB story explicit so we do not carry an implied
“fallback backend” that cannot actually be used by the boot-critical Rust device models.

---

### Why a sync wrapper around IndexedDB deadlocks (in the same Worker)

IndexedDB operations complete by delivering events (`onsuccess` / `onerror`) to the **same
agent’s event loop** that initiated the request:

1. The code issues an IDB request (e.g. `store.get(key)`).
2. The browser schedules an event to run later on that Worker’s event loop.
3. The event fires and your callback resolves a Promise / wakes a future.

If a Rust synchronous API (like `StorageBackend::read_at`) tries to:

- start an IndexedDB request, and then
- **block the current thread** until the callback resolves,

the callback can never run because the Worker’s event loop is blocked by the “sync wait”.

This deadlock happens regardless of *how* you block:

- A busy loop/spin-wait prevents the event loop from running.
- `Atomics.wait()` blocks the Worker thread entirely (no event loop progress).
- `futures::executor::block_on()`-style loops can repeatedly poll a future, but the future
  will remain `Pending` forever because the wakeup is delivered by an IDB callback that can’t run.

The key rule:

> **You cannot synchronously wait for IndexedDB from the same Worker that must run the IndexedDB
> completion callbacks.**

The only way to provide a *synchronous* facade is to run IndexedDB on a **different** Worker (with
its own event loop) and use cross-worker signaling to wait for results (see Option C).

---

### Viable options for Aero

This section enumerates the realistic integration paths. The goal is not to pick the “perfect”
architecture, but to be explicit about constraints so `ST-005` doesn’t accidentally promise a
fallback that cannot back the Rust controller.

#### A) Require OPFS for the Rust controller path; IndexedDB only for disk manager/UI/import/export

**Idea:** Keep `aero-storage` + Rust controllers synchronous and boot-critical; in browser builds,
only allow the Rust controller path when **OPFS SyncAccessHandle** is available. IndexedDB remains
available for:

- disk manager / UI metadata
- import/export staging areas
- snapshots saved outside the boot-critical hot path
- benchmarks / diagnostics

**Pros**

- Minimal architectural change; fits current Rust sync traits.
- Best performance (OPFS SyncAccessHandle is the intended fast path).
- Avoids a fragile “sync IndexedDB” hack that would deadlock.

**Cons / constraints**

- Requires OPFS (and specifically SyncAccessHandle) to run the Rust controller path.
- Browsers/contexts without OPFS SyncAccessHandle must use an alternate mode (or fail early with
  a clear error).
- `ST-005` cannot mean “plug IndexedDB into `aero_storage::VirtualDisk`” under this option; it must
  be scoped to non-controller uses (see recommendation below).

#### B) Move disk controllers to an async worker (TS or Rust/WASM); keep IndexedDB async

**Idea:** Make the *controller* itself async so disk I/O can `await` IndexedDB/async OPFS directly
without blocking.

Concretely: AHCI/IDE command processing becomes an async state machine. When a command needs disk
data, it issues an async read and resumes later to DMA data into guest RAM and raise interrupts.

**Pros**

- IndexedDB fits naturally (no sync wrapper).
- Allows async OPFS APIs and network streaming to share the same model.

**Cons / constraints**

- Large rework of the Rust controller code, which is currently written assuming synchronous I/O.
- Requires careful design to preserve determinism and to avoid starving other device work.
- Complicates the “device model is a pure function of guest-visible state + sync backend” mental model.

#### C) Dedicated async “storage worker” + sync RPC client in the controller worker (SAB + Atomics)

**Idea:** Keep the Rust controller code synchronous, but move IndexedDB access to a **separate**
Worker that is free to `await` IDB callbacks. The controller worker talks to it via a shared-memory
RPC protocol:

- Controller worker (sync): writes a request record into a shared ring buffer, then waits for a
  completion (e.g. `Atomics.wait` on a per-request slot).
- Storage worker (async): reads requests, performs async IndexedDB operations, writes results back,
  and signals completion.

**Pros**

- Preserves the synchronous Rust controller API surface (`VirtualDisk` stays sync).
- Allows IndexedDB to be a true fallback backend for the Rust stack (without deadlocking).
- Similar patterns already exist in Aero (SharedArrayBuffer rings for high-frequency IPC).

**Cons / constraints**

- Requires `SharedArrayBuffer` + `Atomics` (i.e. cross-origin isolation) for the sync wait.
- Requires designing and maintaining a new shared-memory RPC protocol (buffer ownership, backpressure,
  cancellation, timeouts, poisoning/recovery).
- Still adds cross-worker copy/serialization overhead for data buffers unless designed carefully.

#### D) Introduce async traits in Rust (`AsyncStorageBackend` / async controllers)

**Idea:** Make `aero-storage` async end-to-end, and then adapt controllers (and any callers) to
`async fn read_at(...)` / `async fn write_at(...)`.

**Pros**

- Most “pure” from an API standpoint: async storage is represented as async storage.
- Removes the need for sync blocking and reduces the need for cross-worker sync RPC tricks.

**Cons / constraints**

- Large refactor across `crates/aero-storage`, `crates/aero-devices-storage`, tests, and any glue code.
- Requires an async runtime model inside the emulation workers.
- High risk of scope creep and integration churn during boot-critical bring-up.

---

### Recommendation (near-term)

**Choose Option A for the near term.**

Rationale:

- The Rust AHCI/IDE device models are on the Windows 7 boot-critical path. We should optimize for
  correctness and simplicity first.
- OPFS SyncAccessHandle is the only browser storage API that is both (a) performant enough for large
  random I/O and (b) *actually synchronous* inside a Worker.
- Making the Rust controller async (Option B/D) or introducing a new SAB-based storage RPC layer
  (Option C) are both real, but substantially larger projects than the intended scope of `ST-005`.

How this ties back to the **IndexedDB fallback backend** work:

- **ST-005 should be treated as an async-only fallback backend** that is usable by the **web host
  layer / disk manager / import-export tooling**, *not* as an implementation of
  `aero_storage::StorageBackend` / `aero_storage::VirtualDisk`.
- If we later decide we need IndexedDB as a runtime fallback for the Rust controller path, that is
  a separate milestone and likely maps to **Option C**.

---

### Follow-up implementation tasks (for Option A)

Concrete tasks to make this decision “real” in the codebase (so future work doesn’t accidentally
reintroduce an impossible fallback):

1. **Make OPFS SyncAccessHandle a hard requirement for the Rust controller runtime**
   - Web runtime: gate the “Rust controller” boot path on SyncAccessHandle availability and fail
     early with a clear user-facing error.
   - Likely targets:
     - `apps/web/src/runtime/coordinator.ts` (capability checks + selecting runtime mode)
     - `apps/web/src/platform/features.ts` (feature report surface)

2. **Document in Rust APIs that IndexedDB does not implement the synchronous traits**
   - Add or strengthen doc-comments (no behavior changes) clarifying:
     - `crates/aero-storage/src/backend.rs` (`StorageBackend` is sync; IDB is async-only)
     - `crates/aero-opfs/src/io/storage/backends/opfs.rs` (`OpfsIndexedDbBackend` is async-only and
       cannot back `aero_storage::VirtualDisk`)

3. **Scope ST-005 to the async host layer**
   - Use `crates/st-idb` (Rust/wasm32) and/or `apps/web/src/storage/indexeddb.ts` (TypeScript) for:
     - disk/image management UIs
     - import/export staging
     - snapshot storage (when not on the hot I/O path)
   - Ensure any UI/runtime “fallback” messaging is explicit that IndexedDB does **not** enable the
     synchronous Rust controller path.

4. **Add a “no implicit IndexedDB fallback for Rust controllers” test/guardrail**
   - A lightweight guardrail can be a unit/integration test that asserts the Rust controller
     boot path only accepts `aero_opfs::OpfsBackend`/`OpfsByteStorage` (or other truly synchronous
     backends), never `OpfsStorage::IndexedDb`.
   - Likely targets:
     - `crates/aero-devices-storage/tests/*` (controller integration tests)
     - Any future wasm boot harness once the Rust controller is wired into the web runtime.

## Storage trait consolidation (disk/backing-store traits)

### Context

This repo currently contains **multiple overlapping “disk/backing-store” traits** across Rust and
TypeScript. That has been useful during bring-up, but it also creates a constant risk that new work
accidentally introduces *yet another* incompatible trait instead of reusing existing ones.

This document is the repo’s **source of truth** for:

1. What disk/backing-store traits exist today (inventory, with links)
2. Which traits are **canonical** for each layer
3. How adapters should be structured (where the wrapper types vs `impl Trait for ...` live)
4. The browser story (sync vs async, IndexedDB deadlock constraints)

See also:

- [`05-storage-subsystem.md`](storage.md) (subsystem overview)
- [`19-indexeddb-storage-story.md`](storage.md) (the async-vs-sync browser constraint analysis)
- [`platform-and-firmware.md`](platform-and-firmware.md) (the machine and device stack, and the account of the retired second stack)

---

### Inventory: disk/backing-store traits in-tree

> Links are to the defining source files.

#### Rust (synchronous)

- `aero_storage::StorageBackend` (sync, **byte-addressed**, resizable backend)\
  Defined in: [`crates/aero-storage/src/backend.rs`](../../crates/aero-storage/src/backend.rs)
- `aero_storage::StdFileBackend` / `aero_storage::FileBackend` (sync, byte-addressed native `std::fs::File` backend; non-wasm32)\
  Defined in: [`crates/aero-storage/src/backend.rs`](../../crates/aero-storage/src/backend.rs)
- `aero_storage::ReadOnlyBackend` (sync, read-only wrapper for `aero_storage::StorageBackend`)\
  Defined in: [`crates/aero-storage/src/backend.rs`](../../crates/aero-storage/src/backend.rs)
- `aero_storage::VirtualDiskSend` (sync, helper trait: `Send` on native, empty on wasm32; used to make `VirtualDisk` conditionally `Send`)\
  Defined in: [`crates/aero-storage/src/disk.rs`](../../crates/aero-storage/src/disk.rs)
- `aero_storage::VirtualDisk` (sync, fixed-capacity virtual disk; byte addressing + sector helpers; `Send` on native, may be `!Send` on wasm32)\
  Defined in: [`crates/aero-storage/src/disk.rs`](../../crates/aero-storage/src/disk.rs)
- `aero_storage::ReadOnlyDisk` (sync, read-only wrapper for `aero_storage::VirtualDisk`)\
  Defined in: [`crates/aero-storage/src/disk.rs`](../../crates/aero-storage/src/disk.rs)
- `aero_storage::ChunkStore` (sync, chunk-addressed cache store used by `aero_storage::{StreamingDisk, ChunkedStreamingDisk}`; **native-only** / non-wasm32)\
  Defined in: [`crates/aero-storage/src/streaming.rs`](../../crates/aero-storage/src/streaming.rs)
- `aero_devices::storage::DiskBackend` (sync, byte-addressed device-model backend used by the `aero-devices` device stack, including its virtio-blk model)\
  Defined in: [`crates/devices/src/storage/mod.rs`](../../crates/devices/src/storage/mod.rs)
- `aero_devices_storage::atapi::IsoBackend` (sync, read-only 2048-byte-sector CD/ISO backend for ATAPI)\
  Defined in: [`crates/aero-devices-storage/src/atapi.rs`](../../crates/aero-devices-storage/src/atapi.rs)
- `aero_io_snapshot::io::storage::state::DiskBackend` (sync, byte-addressed backend used by the snapshot layer)\
  Defined in: [`crates/aero-io-snapshot/src/io/storage/state.rs`](../../crates/aero-io-snapshot/src/io/storage/state.rs)
- `aero_opfs::io::snapshot_file::OpfsSyncFileHandle` (sync, byte-addressed file-handle interface used to adapt OPFS `SyncAccessHandle` to `std::io::{Read, Write, Seek}`)\
  Defined in: [`crates/aero-opfs/src/io/snapshot_file.rs`](../../crates/aero-opfs/src/io/snapshot_file.rs)
- `firmware::bios::BlockDevice` (sync, 512-byte-sector read-only block device interface used by the legacy BIOS INT 13h implementation)\
  Defined in: [`crates/firmware/src/bios/mod.rs`](../../crates/firmware/src/bios/mod.rs)

#### Rust (asynchronous, browser-oriented)

- `st_idb::io::storage::DiskBackend` (async, byte-addressed backend used by the IndexedDB block store + cache)\
  Defined in: [`crates/st-idb/src/io/storage/mod.rs`](../../crates/st-idb/src/io/storage/mod.rs)

#### Rust (asynchronous, server-side)

- `aero_storage_server::store::ImageStore` (async, byte-stream access for serving disk images over HTTP)\
  Defined in: [`crates/aero-storage-server/src/store/mod.rs`](../../crates/aero-storage-server/src/store/mod.rs)

#### TypeScript (asynchronous, browser-oriented)

- `AsyncSectorDisk` (async, sector-addressed)\
  Defined in: [`apps/web/src/storage/disk.ts`](../../apps/web/src/storage/disk.ts)
- `SparseBlockDisk` (async, sector-addressed + fixed-size block operations for sparse/overlay disks)\
  Defined in: [`apps/web/src/storage/sparse_block_disk.ts`](../../apps/web/src/storage/sparse_block_disk.ts)
- `RemoteRangeDiskSparseCache` / `RemoteRangeDiskSparseCacheFactory` / `RemoteRangeDiskMetadataStore` (async, remote Range disk cache abstractions)\
  Defined in: [`apps/web/src/storage/remote_range_disk.ts`](../../apps/web/src/storage/remote_range_disk.ts)
- `BinaryStore` (async, byte store abstraction used by the chunked remote disk implementation)\
  Defined in: [`apps/web/src/storage/remote_chunked_disk.ts`](../../apps/web/src/storage/remote_chunked_disk.ts)
- `DiskAccessLease` (async, refreshable “signed URL lease” used by remote disk readers)\
  Defined in: [`apps/web/src/storage/disk_access_lease.ts`](../../apps/web/src/storage/disk_access_lease.ts)
- `RemoteCacheDirectoryHandle` / `RemoteCacheFileHandle` / `RemoteCacheFile` / `RemoteCacheWritableFileStream` (async, OPFS-like handle abstractions used by the remote cache manager)\
  Defined in: [`apps/web/src/storage/remote_cache_manager.ts`](../../apps/web/src/storage/remote_cache_manager.ts)
- `RemoteChunkCacheBackend` (async, chunk cache backend interface for OPFS LRU chunk caches)\
  Defined in: [`apps/web/src/storage/remote/opfs_lru_chunk_cache.ts`](../../apps/web/src/storage/remote/opfs_lru_chunk_cache.ts)
- `AeroIpcIoDispatchTarget` (AIPC I/O server dispatch interface; includes optional disk read/write RPC hooks)\
  Defined in: [`apps/web/src/io/ipc/aero_ipc_io.ts`](../../apps/web/src/io/ipc/aero_ipc_io.ts)

---

### Intended layering (what to use where)

The storage stack is easiest to reason about when split into explicit layers:

1. **Host persistence** (OPFS file, IndexedDB, in-memory, network)
2. **Byte backend** (random access, resize)
3. **Disk image formats / disk wrappers** (raw, sparse, qcow2, VHD, block cache, COW overlays)
4. **Device/controller integration** (AHCI/IDE/NVMe/virtio-blk consuming a “disk”)

#### Layer 2 (byte backend): canonical = `aero_storage::StorageBackend`

Use `aero_storage::StorageBackend` as the canonical Rust trait for **synchronous byte-addressed**
storage (files, buffers, OPFS sync access handles).

This is the trait that disk image formats should be generic over.

Examples of implementations:

- `aero_storage::MemBackend` (tests)
- `aero_storage::FileBackend` / `aero_storage::StdFileBackend` (native filesystem backend; non-wasm32)\
  Used by host-side tooling such as:
  - [`tools/aero-disk-convert`](../../tools/aero-disk-convert/src/main.rs)
  - [`tools/image-chunker`](../../tools/image-chunker/src/main.rs)
  and CLI/image tests.
- `aero_opfs::OpfsByteStorage` (browser OPFS SyncAccessHandle; wasm32 only)

#### Layer 3 (disk image formats): canonical = `aero_storage::VirtualDisk` (+ `StorageBackend`)

Disk image formats and disk “wrappers” should live in `crates/aero-storage` and should expose a
fixed-capacity, random-access disk via `aero_storage::VirtualDisk`.

Rule of thumb:

- If you are implementing a **disk image format** (raw/qcow2/vhd/sparse/overlay/cache), implement
  `VirtualDisk` and (if it needs a resizable backing store) be generic over `StorageBackend`.
- Do **not** introduce new format-specific “disk traits” in other crates.

##### Backing layers / explicit parent disks

Some disk image formats can reference an external base layer:

- QCOW2 backing files
- VHD differencing disks

For these images, `DiskImage::open_auto` intentionally **rejects** them unless you explicitly supply
the parent/base disk. To open them, use:

- `aero_storage::DiskImage::open_with_parent` / `aero_storage::DiskImage::open_auto_with_parent`, or
- format-specific helpers: `aero_storage::{Qcow2Disk, VhdDisk}::open_with_parent`.

##### Read-only wrappers (when and why)

Aero frequently attaches media that is **semantically immutable**:

- **Base OS images** (golden Windows installs, demo images)
- **Install/driver ISOs** attached via ATAPI
- **Remote “chunked” base images** served from object storage/CDNs

For these, prefer to enforce immutability in the **disk/backing-store layer** with a read-only
wrapper (e.g. `aero_storage::ReadOnlyDisk` / `aero_storage::ReadOnlyBackend` around the canonical
traits), rather than relying on “we just won’t call `write_at`”.

This catches accidental write paths early (and makes bugs loud), and it documents intent at the API
boundary.

Typical composition for a writable VM disk built on a read-only base:

1. **Base (read-only)**: `VirtualDisk` backed by a remote image, a local file, or an unpacked format
   (qcow2/vhd/…)
2. **Writeback overlay (writable)**: `aero_storage::AeroCowDisk` or a sparse disk overlay
3. **Optional cache**: `aero_storage::BlockCachedDisk`
4. **Device integration**: device/controller consumes `Box<dyn aero_storage::VirtualDisk>`

##### Aero sparse formats: `AEROSPAR` vs legacy `AEROSPRS`

- `AEROSPAR` (“AeroSparse”, magic `AEROSPAR`) is the **current** sparse disk format implemented in
  `crates/aero-storage` as `aero_storage::AeroSparseDisk` in
  [`crates/aero-storage/src/sparse.rs`](../../crates/aero-storage/src/sparse.rs).
- `AEROSPRS` (magic `AEROSPRS`) is the **legacy** sparse format, implemented alongside it in
  [`crates/aero-storage/src/sparse_v1.rs`](../../crates/aero-storage/src/sparse_v1.rs).
- Why it exists: open/migrate older images created before `AEROSPAR` became canonical.
- Differences: `AEROSPAR` has a 64‑byte header + simple u64 allocation table; `AEROSPRS` uses a 4 KiB
  header with explicit sector size (512/4096) plus a small journal for crash‑safe table updates.
- Status/limits: nothing creates `AEROSPRS`. It is read to migrate images that already exist; new
  images are `AEROSPAR`.
- Migration: the `aerosparse_convert` binary of `crates/aero-storage`. Only allocated blocks are
  read, and blocks that turn out to be entirely zero are skipped rather than written, so a sparse
  image converts to a sparse image.
- Tests: [`crates/aero-storage/tests/aerosprs_to_aerospar_conversion.rs`](../../crates/aero-storage/tests/aerosprs_to_aerospar_conversion.rs)
  covers the round trip, and [`crates/aero-storage/tests/storage_formats.rs`](../../crates/aero-storage/tests/storage_formats.rs)
  covers detection.

#### Layer 4 (synchronous device/controller models): canonical = `aero_storage::VirtualDisk`

New synchronous Rust device/controller models should treat `aero_storage::VirtualDisk` as the
canonical “disk” trait.

Some device crates currently define their own `DiskBackend` traits for historical reasons (custom
error types, sector-size reporting, or different mutability requirements). Those traits should be
treated as **device-internal integration traits**, and code should prefer accepting a
`Box<dyn aero_storage::VirtualDisk>` at public boundaries unless there is a concrete reason not to.

In particular, the `aero-devices` virtio stack exposes a crate-local backend trait
(`aero_devices::storage::DiskBackend`). Treat that trait as *device-internal*; “platform wiring”
should prefer `aero_storage::VirtualDisk` and adapt at the device boundary.

The NVMe controller model (`crates/aero-devices-nvme`) has been migrated to consume
`aero_storage::VirtualDisk` directly (no crate-local disk trait).

`aero_virtio`’s virtio-blk implementation consumes `aero_storage::VirtualDisk` directly (no extra
backend trait).

When a device crate *must* keep its own trait, the preferred integration pattern is:

- Accept the device’s trait internally (e.g. `aero_devices::storage::DiskBackend`)
- Provide an adapter from `aero_storage::VirtualDisk` at the API boundary (e.g.
  `aero_devices::storage::VirtualDrive::try_new_from_aero_virtual_disk`)

---

### Adapter structure (important: where code should live)

#### Wrapper types live in `crates/aero-storage-adapters`

`crates/aero-storage-adapters` provides wrapper **types** around `aero_storage::VirtualDisk`
(e.g. mutex/refcell wrappers, alignment enforcement, helper accessors).

This avoids duplicating the same wrapper type across multiple device crates.

#### Trait `impl`s live in the trait’s owning crate (Rust orphan rules)

Because of Rust’s orphan rules, `impl SomeExternalTrait for SomeExternalType` must live in a crate
that defines either the trait or the type.

Therefore:

- `aero-storage-adapters` should generally **not** implement `aero_devices::*::DiskBackend` traits.
- The crate that defines the trait (e.g. `aero-devices-nvme`, `aero-devices`) should implement its
  trait for the wrapper types provided by `aero-storage-adapters`.

This pattern is already used today:

 - Wrapper types: `crates/aero-storage-adapters`
 - `impl aero_devices::storage::DiskBackend for AeroVirtualDiskAsDeviceBackend` (re-exported as
   `aero_devices::storage::AeroStorageDiskAdapter`): in `crates/devices`
 - virtio-blk (`crates/aero-virtio`) consumes `Box<dyn aero_storage::VirtualDisk>` directly\
   Note: on native, `aero_storage::VirtualDisk: Send` (via `VirtualDiskSend`), so `Box<dyn VirtualDisk>`
   is already `Send`; on wasm32 builds it may be `!Send`.
 - Reverse adapter: `crates/devices/src/storage/mod.rs` defines `DeviceBackendAsAeroVirtualDisk`, which
   allows reusing `aero-storage` disk wrappers (cache/sparse/COW) on top of an existing
   `aero_devices::storage::DiskBackend`.
 - BIOS/firmware bridge: `crates/aero-machine/src/shared_disk.rs` defines `SharedDisk`, a wrapper type
   that implements both `firmware::bios::BlockDevice` and `aero_storage::VirtualDisk` so a single
   disk image can be used consistently across the “boot firmware” and “PCI storage controller”
   phases.

If a new device crate needs a disk backend trait, it should follow the same structure:

1. Prefer `aero_storage::VirtualDisk` directly
2. If a crate-specific trait is still needed, add:
   - a wrapper type (if required) in `aero-storage-adapters`
   - the trait `impl` in the device crate

---

### Browser story (sync vs async): Option A vs Option C

This repo has to bridge two realities:

- Rust device/controller models are currently **synchronous**
- IndexedDB (and many other browser APIs) are fundamentally **asynchronous**

`storage.md` explains why “pretend IndexedDB is sync” deadlocks if attempted
from the same Worker.

#### Option A (near-term default): require OPFS SyncAccessHandle for the Rust controller path

Use OPFS `FileSystemSyncAccessHandle` (worker-only) for boot-critical synchronous disk I/O.
IndexedDB remains available for **async-only host layer** features:

- disk manager metadata and UI
- import/export staging
- benchmarks/diagnostics
- snapshots saved outside the boot-critical hot path

This is the current recommendation in `storage.md`.

#### Option C (future / when truly needed): dedicated “storage worker” + sync RPC client

If we need IndexedDB to be a runtime fallback for the synchronous Rust controller path, the only
viable approach is Option C:

- Run IndexedDB I/O on a **separate worker** (async)
- Expose a synchronous facade to the controller worker via a shared-memory RPC protocol

This repo already has building blocks for such shared-memory “sync RPC” in the web runtime (AIPC
ring buffers + `Atomics.wait`), e.g. `apps/web/src/io/ipc/aero_ipc_io.ts`. Option C should reuse those
primitives rather than introducing a new parallel IPC mechanism.

Under Option C, the end goal is still to implement the **canonical sync traits**
(`aero_storage::StorageBackend` / `aero_storage::VirtualDisk`) on top of the RPC client.

This avoids introducing new “special IndexedDB sync traits” and keeps the controller code unchanged.

---

### Canonical traits (quick reference)

- Disk image formats (Rust): **`aero_storage::{StorageBackend, VirtualDisk}`**
- Synchronous device/controller models (Rust): **`aero_storage::VirtualDisk`** (adapt as needed)
- Async browser host layer:
  - Rust/wasm helper crates (IndexedDB cache): **`st_idb::io::storage::DiskBackend`**
  - TypeScript runtime disks: **`AsyncSectorDisk`**

---

### Phased migration plan (to reduce/remove legacy traits)

This is intentionally a **sequence of small PRs** (no repo-wide flag day).

#### Phase 0 (this PR): document + guardrails

1. Add this doc (`storage.md`)
2. Add doc-comments on legacy traits pointing to the canonical traits
3. Add warnings on “fallback” constructors that may produce async-only backends (browser)

#### Phase 1: converge disk image formats on `crates/aero-storage` — done

There is exactly one place in-tree implementing raw, qcow2, VHD, sparse, and overlay logic. The
second implementation went with the device stack that carried it; the one format it had that
`aero-storage` did not — `AEROSPRS` — moved across rather than being dropped.

#### Phase 2: converge device/controller models on `aero_storage::VirtualDisk`

1. Prefer passing `Box<dyn aero_storage::VirtualDisk>` through “platform wiring” layers.
2. Limit crate-specific `DiskBackend` traits to truly internal needs.
3. Keep `crates/aero-storage-adapters` as the shared home for adapter wrapper *types*.
4. Standardize on consistent adapter naming and provide adapters in both directions:
   - `VirtualDisk` → device backend: device crates typically re-export the canonical wrapper types
     as `AeroStorageDiskAdapter` (e.g. `aero_devices::storage::AeroStorageDiskAdapter`).
   - Device backend → `VirtualDisk`: reverse adapters live in the device crates (e.g.
     `aero_devices::storage::DeviceBackendAsAeroVirtualDisk`).
5. virtio-blk: `aero_virtio` is already consolidated on `aero_storage::VirtualDisk`. Keep
   `VirtualDisk` as the wiring boundary and treat any remaining backend traits as device-internal
   (adapt at the edge). Concretely, prefer wiring:
    - `aero_virtio::devices::blk::VirtioBlk` (consumes `Box<dyn aero_storage::VirtualDisk>`)
    - `aero_devices::storage::VirtualDrive::{new_from_aero_virtual_disk, try_new_from_aero_virtual_disk}`
      (for the `aero-devices` stack; prefer `try_new_*` when accepting arbitrary disks)

   (Optional cleanup) Evaluate consolidating any remaining virtio-blk device models on fewer backend
   traits (e.g. `aero_devices::storage::DiskBackend`). In all cases, keep
   `aero_storage::VirtualDisk` as the common “disk image” boundary and adapt.

#### Phase 3: remove the parallel storage traits — done

The second stack's `ByteStorage` and `DiskBackend` are gone, along with the adapters that existed
only to convert between them and `aero_storage::StorageBackend` and `VirtualDisk`.

One thing did not survive the move, and is worth naming so the choice is re-examinable: a
coalescing backend that merged adjacent scatter-gather ranges, and a block cache with a write-back
policy and incremental flush. `aero-storage`'s cache has neither, and nothing in the tree merges
scatter-gather ranges. Adopting the coalescer means adding vectored I/O to `VirtualDisk` — a change
to the canonical trait rather than a migration — so it waits for someone who needs it. The code is
recoverable from `.attic/legacy-crates/`.

#### Phase 4 (optional, future): IndexedDB as runtime fallback for sync controllers (Option C)

1. Implement the “storage worker” RPC protocol (shared memory + `Atomics`)
2. Implement `aero_storage::StorageBackend` for the RPC client
3. Reuse existing `aero_storage` disk formats + existing synchronous controllers unchanged
