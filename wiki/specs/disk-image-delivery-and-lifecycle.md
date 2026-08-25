# Disk image delivery and lifecycle

> Getting a multi-gigabyte Windows image to a browser tab and keeping access to
> it controlled: image management, ranged and chunked delivery, the
> content-delivery behaviour the fast path depends on, streaming
> authorisation, and the upload-to-writeback lifecycle.
>
> The on-disk container used by the no-range path is specified in
> [chunked-disk-image-format.md](./chunked-disk-image-format.md).

## Overview

This repository now includes a minimal, browser-side **disk image manager** intended for real users:

- Create blank HDD images of a configurable size (e.g. 20–60GB).
- Import existing images (`.img`, `.iso`, `.qcow2`) via streaming `File.stream()` into OPFS (preferred) or IndexedDB (fallback).
- Import and convert common disk images (`.img`/raw, `.qcow2`, `.vhd`) into Aero’s internal sparse-on-OPFS format (`AEROSPAR`; implemented in [`crates/aero-storage/src/sparse.rs`](../../crates/aero-storage/src/sparse.rs); status/migration notes in [`20-storage-trait-consolidation.md`](../areas/storage.md)) via `DiskManager.importDiskConverted()` (OPFS-only).
- Export images as a `ReadableStream<Uint8Array>` (or via `DiskManager.exportDiskToFile()`), with optional gzip compression when `CompressionStream` is available.
- Persist metadata (JSON) per backend: name, kind (HDD/CD), size, last used, and a streaming CRC32 checksum.
- Maintain a mount selection: **one HDD (rw)** + **one CD (ro)** at minimum.

Note: IndexedDB storage is async and is used here as a host-side fallback for disk management and
import/export flows. It is not currently exposed as a synchronous `aero_storage::StorageBackend` /
`aero_storage::VirtualDisk` for the boot-critical Rust controller path; see
[`19-indexeddb-storage-story.md`](../areas/storage.md) and
[`20-storage-trait-consolidation.md`](../areas/storage.md).

Implementation lives in:

- `apps/web/src/storage/disk_manager.ts` (main-thread API, worker client)
- `apps/web/src/storage/disk_worker.ts` (worker implementation)
- `apps/web/src/storage/import_export.ts` (streaming IO, OPFS/IDB)
- `apps/web/src/storage/metadata.ts` (metadata + mounts)

## Storage layout

### OPFS backend

- Root: `navigator.storage.getDirectory()`
- Disk directory: `aero/disks/`
  - Images: `<diskId>.<ext>` (e.g. `.../aero/disks/<uuid>.img`)
  - Metadata: `aero/disks/metadata.json`

### IndexedDB backend

Single database: `aero-disk-manager`

- `disks` store: per-disk metadata
- `mounts` store: current mount selection
- `chunks` store: raw data chunks keyed by `[diskId, index]` (sparse; missing chunks are treated as zeros)

## Manual checklist (large import w/ progress)

1. Open the app with the disk manager UI (or a dev harness calling `DiskManager.importDisk()`).
2. Choose the OPFS backend if available.
3. Import a large image (multi-GB `.img` or `.iso`).
4. Verify:
   - Progress updates are displayed continuously (bytes processed increases).
   - The UI remains responsive during import (no main-thread hangs).
   - After import, the image appears in `DiskManager.listDisks()` with correct `sizeBytes`.
   - A checksum is recorded for imported images (`metadata.checksum.algorithm === "crc32"`).
5. Export the imported image:
   - Verify that export progress updates and the UI remains responsive.
   - Verify the exported file checksum matches metadata (or matches the original file if available).

## Remote Disk Image Delivery (Object Store + CDN + HTTP Range)

### Overview

Aero supports **streaming 20GB+ disk images** into the browser without downloading the whole file up front. The core idea is:

- The browser exposes a block-device-like API (`StreamingDisk`) that performs **random-access reads**.
- Reads are satisfied by issuing `GET` requests with an `HTTP Range` header against an immutable disk image stored in an **S3-compatible object store**.
- A **CDN** (CloudFront is the primary reference) sits in front of the object store to cache hot ranges and reduce origin load.
- Downloaded ranges are persisted locally (OPFS) so repeated reads stop hitting the network.

Implementation note: in Rust (native/non-wasm32), the streaming-disk implementation and cache
traits live under `crates/aero-storage` (e.g. `aero_storage::StreamingDisk` /
`aero_storage::ChunkStore`).
For the repo-wide canonical disk/backend trait mapping, see
[`20-storage-trait-consolidation.md`](../areas/storage.md).

This document defines the **production contract** for the `remote_url` used by `StreamingDisk` and provides deployment guidance (caching, CORS, security).

For the normative auth + CORS + COOP/COEP behavior of the disk bytes endpoint, see: [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md).

If you want a CDN-friendlier alternative that avoids `Range` (and therefore avoids `Range`-triggered CORS preflight in cross-origin deployments), see: [Chunked Disk Image Format](chunked-disk-image-format.md).

### Reference infrastructure in this repo

- Local development (MinIO + optional reverse proxy for edge/CORS emulation):
  - [`infra/local-object-store/README.md`](../decisions/README.md)
- AWS production reference (S3 + CloudFront tuned for Range + CORS):
  - [`infra/aws-s3-cloudfront-range/README.md`](../decisions/README.md)
- Reference backend for user-uploaded/private images (S3 multipart upload + CloudFront signed access):
  - ``services/image-gateway/``

Companion tools:

- Correctness + CORS conformance checks: [`tools/disk-streaming-conformance/`](../decisions/README.md)
- Range throughput + CDN cache probing (`X-Cache`): [`tools/range-harness/`](../decisions/README.md)
- Chunked disk publisher + verifier (no-`Range` delivery): [`tools/image-chunker/`](../decisions/README.md)
  - `publish --format <raw|qcow2|vhd|aerosparse|auto>` (alias: `aerospar`): publish the logical disk byte stream for common formats.
  - `verify`: validate a published `manifest.json` + `chunks/*.bin` end-to-end (supports S3-backed verification, direct HTTP via `--manifest-url`, and local verification via `--manifest-file`).

---

### End-to-end architecture

```
┌───────────────────────────────────────────────────────────────────────────┐
│ Browser (Aero)                                                            │
│                                                                           │
│  Storage stack:                                                          │
│   VirtualDrive → StreamingDisk → (OPFS AEROSPAR sparse cache)             │
│                                                                           │
│  Network behavior:                                                       │
│   1) HEAD remote_url  -> discover Content-Length + ETag/Last-Modified     │
│   2) GET Range bytes=a-b -> fetch aligned CHUNK_SIZE blocks as needed     │
│                                                                           │
└───────────────────────────────┬───────────────────────────────────────────┘
                                │ HTTPS (GET/HEAD/OPTIONS)
                                ▼
┌───────────────────────────────────────────────────────────────────────────┐
│ CDN (CloudFront / equivalent)                                             │
│                                                                           │
│ - Optionally enforces authorization (signed cookies/URLs, JWT at edge, …) │
│ - Caches immutable objects and/or aligned byte ranges                      │
│ - Forwards Range requests to origin                                        │
└───────────────────────────────┬───────────────────────────────────────────┘
                                │ Origin fetch (Range-aware)
                                ▼
┌───────────────────────────────────────────────────────────────────────────┐
│ Object store (S3-compatible)                                              │
│                                                                           │
│ - Stores disk image blobs (20GB+)                                          │
│ - Must support HEAD and Range GET (206 + Content-Range)                    │
│ - Provides ETag/Last-Modified for cache validation                         │
└───────────────────────────────────────────────────────────────────────────┘
```

Key design principle: **the disk image URL should be stable and immutable** (versioned path). This enables aggressive CDN caching and makes local OPFS caches safe.

Implementation note: in the current browser stack, the OPFS cache format is Aero sparse (`AEROSPAR`)
as implemented by [`apps/web/src/storage/opfs_sparse.ts`](../../apps/web/src/storage/opfs_sparse.ts).

---

### Deployment modes

#### Mode A: Public/shared images (demo/test OS images)

Use this for images that are identical for many users (e.g., a Windows 7 demo image).

Goals:

- Maximize cache sharing across users.
- Keep URLs stable and cacheable for a long time.
- Avoid any user-specific cache keys.

Recommended characteristics:

- Immutable, versioned key (e.g. `/images/win7-sp1-x64/2026-01-10/disk.img`)
- `Cache-Control: public, max-age=31536000, immutable, no-transform`
- No authorization required (or optional “soft gating” on the app side; see below)

#### Mode B: Private per-user images (user uploads)

Use this for images that must not be shared across users.

For the full hosted-service model (upload/import flows, ownership/visibility/sharing, and writeback strategies), see: [Disk Image Lifecycle and Access Control](../areas/storage.md).

Goals:

- Strong access control (only the owning user can fetch ranges).
- Prevent URL guessing from becoming a data leak.
- Still leverage CDN performance features (TLS, edge POPs, origin shielding), even if cache sharing is minimal.

Recommended characteristics:

- Objects stored under a per-user prefix, but **authorization must still be enforced**:
  - `/users/<userId>/images/<imageId>/<version>/disk.img`
- Authorization enforced at the CDN (signed cookies/URLs, or an edge auth layer).
- Conservative caching (often short TTL), unless you intentionally want CDN caching for repeat access.

> Note: “Private” does not necessarily mean “uncacheable”.
>
> If the CDN enforces access (e.g., CloudFront signed cookies), it is still safe to use
> `Cache-Control: public` (ideally with `no-transform`) on a private object. The CDN will store bytes, but only serve them
> to authorized viewers. This is useful for **private-but-shared** images (e.g., paid tier),
> and can still be acceptable for per-user images if your threat model allows it.

---

### Why HTTP Range (and why fixed chunk alignment matters)

#### Random access is required

Windows will perform a lot of small reads spread across the disk image:

- filesystem metadata (NTFS MFT, directories)
- pagefile reads/writes (depending on configuration)
- DLL and executable paging
- registry hive access
- boot-time file access patterns

Downloading a 20–40GB disk image before boot is not viable. `StreamingDisk` therefore reads only the bytes it needs, on demand.

#### `StreamingDisk` uses fixed-size aligned chunks

`StreamingDisk` maps arbitrary reads into **aligned fixed-size chunks** (see `CHUNK_SIZE` in the storage doc). Benefits:

- Improves CDN cache hit rate (many clients will request identical ranges).
- Reduces request fan-out by batching adjacent reads.
- Keeps local storage simple (direct mapping to the local sparse-cache block size, i.e. `AEROSPAR`
  blocks).

#### Recommended default `CHUNK_SIZE`

Recommended starting point:

- `CHUNK_SIZE = 1 MiB` (1,048,576 bytes)

Rationale:

- Small enough to avoid excessive over-fetch on random reads.
- Large enough that request overhead (headers, latency, TLS, CDN processing) is amortized.
- Aligns well with `AEROSPAR` block sizes that are typically ~1 MiB.
- Creates a manageable number of total chunks:
  - 20 GiB image ≈ 20,480 chunks
  - 40 GiB image ≈ 40,960 chunks

#### Chunk size tuning

`CHUNK_SIZE` is a trade-off between request rate and over-fetch:

| If you choose… | You get… | You risk… |
|---|---|---|
| Smaller chunks (128–512 KiB) | Less wasted bandwidth on scattered reads | Higher request rate, higher CDN/S3 request costs, more CPU overhead |
| Larger chunks (2–8 MiB) | Fewer requests, better throughput | More wasted bandwidth, larger OPFS footprint, slower “first useful byte” |

Operational guidance:

- Start at **1 MiB**.
- If you see high request rate / high RTT overhead (especially on mobile), try **2 MiB**.
- If you see significant wasted transfer vs. useful bytes (many “cold” reads), try **512 KiB**.
- Keep `CHUNK_SIZE` a power-of-two and a multiple of 512 bytes.

---

### HTTP semantics: required behavior

#### HEAD for size discovery and versioning

Before range reads, `StreamingDisk` needs the object size and a stable validator (ETag/Last-Modified).

Requirements:

- `HEAD <remote_url>` must return:
  - `200 OK`
  - `Content-Length: <total-bytes>`
  - `Accept-Ranges: bytes` (or `Accept-Ranges: bytes` on subsequent GETs)
  - `ETag` and/or `Last-Modified`

Example:

```http
HEAD /images/win7/2026-01-10/disk.img HTTP/1.1
Host: aero.example.com
```

```http
HTTP/1.1 200 OK
Content-Length: 21474836480
Accept-Ranges: bytes
ETag: "2f8c3f2a0a4d9b0b0f..."
Last-Modified: Fri, 10 Jan 2026 00:00:00 GMT
Cache-Control: public, max-age=31536000, immutable, no-transform
```

How Aero should use it (conceptually):

- Use `Content-Length` as `total_size`.
- Use `ETag`/`Last-Modified` as a cache key for OPFS:
  - If the validator changes, the local cached chunks must be treated as stale.

#### GET with Range

For chunk fetches, the browser sends:

```http
GET /images/win7/2026-01-10/disk.img HTTP/1.1
Host: aero.example.com
Range: bytes=1048576-2097151
```

Required origin/CDN behavior:

- Respond with `206 Partial Content`
- Include `Content-Range` describing the served range and total size
- Include `Content-Length` equal to the returned byte count
- Include `Accept-Ranges: bytes`
- Include `Cache-Control` containing `no-transform` (Aero clients enforce this to prevent intermediary transforms)

Example response:

```http
HTTP/1.1 206 Partial Content
Content-Type: application/octet-stream
Accept-Ranges: bytes
Content-Range: bytes 1048576-2097151/21474836480
Content-Length: 1048576
Cache-Control: public, max-age=31536000, immutable, no-transform
ETag: "2f8c3f2a0a4d9b0b0f..."
Last-Modified: Fri, 10 Jan 2026 00:00:00 GMT
```

Edge cases:

- If the requested range starts at or beyond the end of the file:
  - Respond with `416 Range Not Satisfiable`
  - Include: `Content-Range: bytes */<total-size>`

```http
HTTP/1.1 416 Range Not Satisfiable
Content-Range: bytes */21474836480
```

> Aero should never intentionally request invalid ranges, but correctness here matters:
> bugs, race conditions, or stale metadata can otherwise degrade into silent data corruption.

#### Cache validators (ETag / Last-Modified)

For immutable, versioned objects:

- `ETag` and `Last-Modified` should remain stable for the lifetime of that version.

For mutable objects (not recommended for disk images):

- Changing bytes must update `ETag` and/or `Last-Modified`.
- Clients and CDNs may revalidate using `If-None-Match` / `If-Modified-Since`.

Important caveat for S3:

- S3 `ETag` is **not guaranteed to be an MD5 checksum**:
  - Multipart uploads typically produce an ETag that encodes part count.
  - Some encryption modes and proxies can also affect ETag semantics.
- Treat `ETag` as a **version/validator**, not a cryptographic integrity hash.
  - If you need an integrity hash, store a separate SHA-256 in metadata or a manifest file.

---

### CORS and preflight (Range is not “simple”)

#### Why `Range` causes a preflight

When `remote_url` is cross-origin relative to the web app, the browser will send an `OPTIONS` preflight because:

- `Range` is not a “simple” request header under the Fetch/CORS rules.

Without mitigation, that means each disk chunk fetch can be preceded by an `OPTIONS` request, which is catastrophic for performance.

#### Mitigations (preferred order)

1. **Same-origin hosting (preferred)**
   - Serve the disk image from the same origin as the app (same scheme/host/port).
   - This avoids CORS entirely and eliminates preflights.
   - With CloudFront, this usually means: one distribution + one domain, with path-based routing.
2. **Aggressive preflight caching**
   - Configure `Access-Control-Max-Age` to a large value (e.g., 86400 seconds).
   - Real-world browsers cap this, but it still helps dramatically.
3. **Chunk-object alternative (if preflight remains a problem)**
   - Pre-split the disk image into fixed-size objects (`chunk_000000`, `chunk_000001`, …).
   - Fetch chunks with plain `GET` (no `Range` header) to keep requests “simple” cross-origin.
   - Costs: many objects, a manifest, more operational complexity.

#### S3 CORS configuration (copy-paste)

If you must fetch cross-origin from S3 (directly or via a CDN that forwards preflights), configure CORS so that:

- `GET`, `HEAD`, and `OPTIONS` are allowed
- request header `Range` is allowed
- response headers needed by Aero are exposed:
  - `Content-Range`, `Accept-Ranges`, `Content-Length`, `ETag`, `Last-Modified`

##### S3 CORS XML example

```xml
<?xml version="1.0" encoding="UTF-8"?>
<CORSConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <CORSRule>
    <AllowedOrigin>https://aero.example.com</AllowedOrigin>
    <AllowedMethod>GET</AllowedMethod>
    <AllowedMethod>HEAD</AllowedMethod>
    <AllowedMethod>OPTIONS</AllowedMethod>
    <AllowedHeader>Range</AllowedHeader>
    <AllowedHeader>If-Range</AllowedHeader>
    <AllowedHeader>If-None-Match</AllowedHeader>
    <AllowedHeader>If-Modified-Since</AllowedHeader>
    <AllowedHeader>Origin</AllowedHeader>
    <AllowedHeader>Access-Control-Request-Method</AllowedHeader>
    <AllowedHeader>Access-Control-Request-Headers</AllowedHeader>
    <ExposeHeader>Accept-Ranges</ExposeHeader>
    <ExposeHeader>Content-Range</ExposeHeader>
    <ExposeHeader>Content-Length</ExposeHeader>
    <ExposeHeader>Content-Encoding</ExposeHeader>
    <ExposeHeader>ETag</ExposeHeader>
    <ExposeHeader>Last-Modified</ExposeHeader>
    <MaxAgeSeconds>86400</MaxAgeSeconds>
  </CORSRule>
</CORSConfiguration>
```

##### S3 CORS JSON example (AWS CLI / IaC friendly)

```json
{
  "CORSRules": [
    {
      "AllowedOrigins": ["https://aero.example.com"],
      "AllowedMethods": ["GET", "HEAD", "OPTIONS"],
      "AllowedHeaders": [
        "Range",
        "If-Range",
        "If-None-Match",
        "If-Modified-Since",
        "Origin",
        "Access-Control-Request-Method",
        "Access-Control-Request-Headers"
      ],
      "ExposeHeaders": [
        "Accept-Ranges",
        "Content-Range",
        "Content-Length",
        "Content-Encoding",
        "ETag",
        "Last-Modified"
      ],
      "MaxAgeSeconds": 86400
    }
  ]
}
```

Notes:

- If you need multiple app origins (staging + prod), add multiple `AllowedOrigin` entries or multiple rules.
- If you use `fetch(..., { credentials: "include" })`, you cannot use `Access-Control-Allow-Origin: *` and must use an explicit origin.

---

### CDN caching strategy (CloudFront reference implementation)

#### Baseline: CloudFront in front of a private S3 bucket (OAC)

Recommended baseline even for “public” images:

- Keep the S3 bucket private (Block Public Access).
- Serve images only through CloudFront.
- Use **Origin Access Control (OAC)** so CloudFront can read from the bucket, but the public internet cannot.

Benefits:

- Avoids direct-to-S3 hotlinking and accidental public exposure.
- Lets you enforce consistent caching/CORS headers at the CDN layer.

#### Signed cookies vs signed URLs (private access)

CloudFront offers two viewer authorization mechanisms that work well for range-based streaming:

- **Signed cookies**
  - Best when the app and disk image share the same origin.
  - Keeps the disk URL stable and cacheable.
  - Authorization is attached to the browser session instead of each URL.
- **Signed URLs**
  - Useful for one-off sharing or when you can’t/won’t set cookies.
  - Signatures are usually query parameters.
  - Important: do **not** include user-specific query parameters in the cache key, or cache sharing is destroyed.

For both mechanisms:

- CloudFront still validates the signature/cookies on every request.
- Cached objects/ranges can be safely reused across authorized viewers if the object itself is not user-specific.

#### Caching and 206 responses

Range fetches return `206 Partial Content`. Your CDN must handle these correctly:

- Forward the `Range` header to the origin.
- Cache must not confuse different ranges for the same object.

In CloudFront, keep the cache key minimal (path + version) and **do not** vary the cache key on `Range`.

- CloudFront supports byte-range requests and can satisfy subsequent ranges from its edge cache.
- Including `Range` in the cache key typically explodes cache cardinality for random access patterns.

If you run your own proxy cache (e.g., Nginx), use a **slice** strategy so the cache stores fixed-size slices and the cache key varies by slice range (not arbitrary viewer ranges). See: [17 - HTTP Range + CDN Behavior](../areas/storage.md).

#### Recommended CloudFront behavior policies (outline)

You can implement this with CloudFront “Policies” (console) or in IaC.

**Cache Policy (for disk images)**

- Query strings: **none** (or only those that are stable and not user-specific)
- Cookies: **none**
- Headers in cache key:
  - (Optional) `Origin` if you serve CORS responses that vary by origin
- TTL:
  - For immutable objects: allow long TTLs (up to 1 year) and respect origin `Cache-Control`
  - For mutable objects: keep TTL low and rely on validators (`ETag`, `Last-Modified`)

**Origin Request Policy**

- Forward to origin:
  - `Range`
  - For CORS preflight: `Origin`, `Access-Control-Request-Method`, `Access-Control-Request-Headers`
- Do not forward unnecessary viewer headers (reduces cache fragmentation and origin load)

**Response Headers Policy**

- Ensure responses include:
  - `Access-Control-Allow-Origin` (if cross-origin; otherwise omit)
  - `Access-Control-Expose-Headers: Content-Range, Accept-Ranges, Content-Length, ETag, Content-Encoding`
    - Note: `Last-Modified` is CORS-safelisted and does not need explicit exposure.
  - `Access-Control-Max-Age` for preflights (if applicable)
- Add security headers as appropriate for your app.

Also verify:

- CloudFront behavior allows `GET`, `HEAD`, and `OPTIONS`.
- “Compress objects automatically” is **disabled** for disk images (or ensure the content-type will never be compressed).

#### “Verify in practice” checklist

Use `curl` against the CDN URL (not the S3 origin) to confirm behavior:

1. HEAD works and returns size + validators:

   ```bash
   curl -I https://aero.example.com/images/win7/2026-01-10/disk.img
   ```

   Check for:

   - `200 OK`
   - `Content-Length`
   - `Accept-Ranges: bytes`
   - `ETag` and/or `Last-Modified`

2. Range GET works and returns 206 + Content-Range:

   ```bash
   curl -I -H 'Range: bytes=0-1048575' \
     https://aero.example.com/images/win7/2026-01-10/disk.img
   ```

   Check for:

   - `206 Partial Content`
   - `Content-Range: bytes 0-1048575/<total>`
   - `Content-Length: 1048576`

3. CDN caching is actually happening:

   Run the same request twice and compare `X-Cache`:

   - First request: `X-Cache: Miss from cloudfront` (expected)
   - Second request: `X-Cache: Hit from cloudfront` (desired)

   Also check `Age` increasing on repeated hits.

4. Cache key sanity:

   - Request two different ranges and ensure you don’t get the same bytes back.
   - Ensure query strings are not unintentionally fragmenting the cache.

5. Observe metrics:

   - CloudFront `CacheHitRate`
   - `BytesDownloadedFromOrigin` (should drop after warm-up for public/shared images)
   - Origin (S3) request counts

---

### Object layout and versioning

#### Use immutable, versioned keys

Recommended key layout:

```
/images/<imageId>/<version>/disk.img
```

Examples:

- `/images/win7-sp1-x64/2026-01-10/disk.img`
- `/images/tiny-linux/2026-01-10/disk.img`

Why:

- Allows **infinite CDN TTLs** without worrying about stale bytes.
- Avoids cache invalidations/purges (which are slow and error-prone).
- Makes client-side OPFS caching safe and simple (validator changes only when version changes).

#### Invalidation strategy

- Publish a new version by writing to a new path.
- Update your application’s “latest version” pointer (often a small JSON manifest with short TTL):

  - `/images/win7-sp1-x64/latest.json` (cache for seconds/minutes)
  - contents point to `/images/win7-sp1-x64/2026-01-10/disk.img`

Avoid:

- Overwriting `disk.img` in-place under a stable URL with long cache headers.
- Relying on CDN “purge” as the normal update mechanism.

---

### Recommended object metadata

Set these on the disk image object (or inject at the CDN):

- `Content-Type: application/octet-stream`
- `Cache-Control`:
  - Immutable versioned objects:
    - `Cache-Control: public, max-age=31536000, immutable, no-transform`
  - Mutable paths (not recommended for disk images):
    - `Cache-Control: no-cache, no-transform` (forces revalidation)
    - or a short TTL (`max-age=60, no-transform`) if you must

Strong warning:

- Do **not** serve disk images with `Content-Encoding: gzip/br` or any automatic compression.
  - Range requests operate on the encoded bytes. Compression changes byte offsets and breaks random-access semantics.
  - Ensure your CDN is not performing “helpful” compression based on incorrect `Content-Type`.

---

### Security and privacy guidance

Threats to consider:

- **Unauthorized guessing** of disk image URLs (especially for per-user objects)
- **Cache confusion** where an authorization token ends up in the cache key or a shared cache serves private data
- **Origin bypass** (direct S3 access) circumventing CDN auth
- **Data at rest exposure** (misconfigured buckets, backups, logs)

Recommended controls:

1. **Use a private bucket + OAC**
   - Block direct public access to S3.
   - Allow reads only from the CDN origin identity.
2. **Enforce auth at the CDN for private images**
   - CloudFront signed cookies/URLs, or an edge auth layer (JWT validation, etc).
3. **Do not put user identity or auth tokens into the cache key**
   - Avoid user-specific query params in cache key.
   - Avoid forwarding cookies/authorization headers into the cache key unless you intend per-user caching.
4. **Object key hygiene**
   - Use non-guessable IDs for `imageId` (UUIDs) even if you also enforce auth.
   - Keep per-user objects under distinct prefixes.
5. **Encrypt at rest**
   - S3 SSE-S3 or SSE-KMS (or equivalent in your object store).
6. **Audit logging**
   - Enable access logs (CloudFront standard logs or equivalent).
   - Monitor for unusual range-scraping patterns.

---

### Cost and performance considerations

#### Request-rate vs chunk size

Range streaming trades bandwidth for requests:

- Requests per GiB ≈ `GiB / (CHUNK_SIZE in GiB)`
  - 1 MiB chunks: ~1024 requests per GiB
  - 4 MiB chunks: ~256 requests per GiB

Costs to watch:

- **Origin request costs** (e.g., S3 charges per GET/HEAD).
- **CDN request costs** (requests and bytes).
- **Cache fragmentation** if ranges are not aligned.

#### S3/object-store request pricing (why alignment matters financially)

Object stores typically bill `GET` and `HEAD` per request. Range requests are usually billed as a normal `GET`.

For a rough order-of-magnitude estimate:

- With `CHUNK_SIZE = 1 MiB`, fetching 1 GiB of unique data is ~1024 range `GET`s (+ a small number of `HEAD`/`OPTIONS`).
- S3 Standard pricing is region-dependent but commonly on the order of **$0.0004 per 1,000 GET requests**.

The important part isn’t the exact number; it’s that:

- Misaligned ranges dramatically reduce CDN cache hit rate, which increases origin requests.
- Good alignment + immutable caching pushes almost all repeated reads onto the CDN and client OPFS cache.

#### CDN egress vs origin egress

For Mode A public/shared images, caching is where the money and performance is:

- First user warms the cache (origin egress).
- Subsequent users hit edge cache (CDN egress, minimal origin egress).

For Mode B per-user images:

- Cache sharing is naturally limited.
- Consider whether CDN caching should be enabled at all; the browser’s OPFS cache already eliminates most repeat reads.

#### Client-side OPFS caching effect

Once a chunk is written to the local `AEROSPAR` sparse cache file in OPFS:

- repeat reads for the same bytes do not incur network cost
- boot performance becomes much less sensitive to CDN cache hit rate after the first run

This makes it reasonable to optimize primarily for **first-boot latency** (good RTT, good throughput) and correct HTTP semantics.

---

### Alternative stacks (what to ensure)

The design works with any object store + CDN combination that supports the required knobs:

- **Cloudflare R2 + Cloudflare CDN**
  - Must support Range GET, correct 206 semantics, and header-based cache key control.
  - Must support a secure origin access pattern (or make R2 private behind the CDN).
- **Fastly + S3 (or other origin)**
  - VCL can explicitly handle `Range` and cache segmentation; ensure correct cache keying.
- **MinIO (self-hosted S3-compatible) + CDN**
  - Verify Range support and HEAD behavior.
  - Ensure consistent ETag/Last-Modified behavior and CORS.

Minimal “must support” checklist:

- `HEAD` returns `Content-Length` for large objects
- `GET` with `Range` returns `206` + correct `Content-Range`
- CDN can forward `Range` to origin
- CDN cache key can vary on `Range` (or otherwise safely cache partial content)
- Ability to disable compression / ensure `Content-Encoding` is not applied
- Ability to serve CORS headers (or host same-origin and avoid CORS)

## Disk Image Streaming (HTTP Range + Auth + COOP/COEP)

### Overview

The Aero emulator frequently needs **random access** to very large disk images (20–40GB+). This document specifies an interoperable, browser-compatible way to fetch disk image bytes from a remote service while:

1. Using HTTP `Range` efficiently (`206 Partial Content`).
2. Enforcing access control for **private** per-user images.
3. Remaining compatible with **cross-origin isolation** (COOP/COEP) required for `SharedArrayBuffer` and WASM threads.
4. Working with CDNs (cacheability, edge authorization, signed URLs/cookies).

This spec is written for both:

* **Client implementers** (browser code that reads disk bytes), and
* **Server/CDN implementers** (the “disk bytes endpoint” that serves ranges).

**Normative language:** The terms **MUST**, **SHOULD**, and **MAY** are used as in RFC 2119.

Related:
- [Storage Subsystem](../areas/storage.md) (client-side streaming disk design)
- [Storage trait consolidation](../areas/storage.md) (repo-wide canonical disk/backend traits)
- [Disk Image Lifecycle and Access Control](../areas/storage.md) (uploads/ownership/sharing/writeback)
- [Remote Disk Image Delivery](../areas/storage.md) (object store + CDN deployment contract)
- [HTTP Range + CDN Behavior](../areas/storage.md) (CloudFront/Cloudflare caching/limits considerations)
- [Chunked Disk Image Format](chunked-disk-image-format.md) (CDN-friendly alternative to `Range`)
- [deployment/cloudfront-disk-streaming.md](../areas/storage.md) (concrete AWS CloudFront setup)
- [`services/image-gateway`](../decisions/README.md) (reference backend: multipart upload + signed CloudFront access)
- [backend/disk-image-streaming-service.md](../areas/storage.md) (ops/runbook: headers, reverse proxy pitfalls, troubleshooting)
- [Browser APIs](../areas/web-host.md) (threads + cross-origin isolation context)

---

### Terminology and threat model

#### Terminology

* **Disk image**: A byte-addressable, immutable (for the duration of a session/lease) representation of a virtual disk. Typically a raw image, qcow2, vhd, or a custom sparse format.
* **Disk bytes endpoint**: An HTTP resource that serves the disk image as a byte stream and supports HTTP `Range` requests. Example: `GET /disks/{diskId}/bytes`.
* **Public image**: An image intended to be readable by anyone who can discover its URL (or identifier), without per-user authorization. Public images are typically CDN-cacheable for long periods.
* **Private image**: An image readable only by an authenticated user (typically the owner) and/or explicitly shared users.
* **Owner**: The principal (user/account) that controls a private image.
* **Sharing** (future): Allowing an owner to delegate read access to another principal, usually by minting a separate capability or share policy. This spec treats sharing as a variation of the same **lease/capability** concept.
* **Disk access lease / capability**: A short-lived, scoped credential authorizing reads of a specific disk image (and optionally specific operations such as `read`, `write`, or a byte-range budget). The lease is represented to the browser as either:
  * a URL containing an embedded token (signed URL), and/or
  * a cookie (session/signed cookie), and/or
  * a header value (e.g., `Authorization: Bearer …`).

The **lease** concept is recommended because it separates:

* the **control plane** (user authentication, authorization decisions, logging), from
* the **data plane** (high-volume `Range` reads, CDN delivery).

#### Threat model

This spec assumes:

* All traffic uses **HTTPS** (TLS). Plain HTTP is not supported.
* Attackers can run arbitrary JavaScript on **other web origins** and attempt to exfiltrate disk bytes.
* Attackers may obtain URLs via:
  * copy/paste and accidental sharing,
  * `Referer` leakage (if query tokens are used),
  * logs/analytics at intermediaries,
  * browser history, or
  * XSS in the application origin (out of scope to fully mitigate).

Primary threats and required mitigations:

1. **Unauthorized read of private images**
   *Private images MUST require a valid lease/capability or equivalent authorization on every request.*
2. **Capability leakage** (especially with query tokens)
   *Leases SHOULD be short-lived, tightly scoped, and revocable (by expiry).*
3. **Cross-origin confusion and CORS bypass**
   *Cross-origin deployments MUST implement correct CORS behavior and MUST NOT rely on “opaque” `no-cors` fetches.*
4. **COOP/COEP violations breaking `SharedArrayBuffer`**
   *All disk fetches used by a crossOriginIsolated app MUST be COEP-compatible (same-origin or explicitly allowed by CORS/CORP).*
5. **Caching private data incorrectly**
   *Private images MUST default to `Cache-Control: no-store, no-transform` unless authorization is enforced at the edge (signed URL/cookie) and cache behavior is intentionally configured.*

---

### Recommended deployment patterns

#### Preferred: same-origin disk endpoints (no CORS / no preflight)

**Goal:** Avoid CORS preflights entirely. `Range` is not a CORS-safelisted request header, so cross-origin requests will preflight unless the resource is same-origin.

Recommended topology:

* App: `https://app.example.com/` (HTML/JS/WASM)
* Disk bytes endpoint: `https://app.example.com/disks/{diskId}/bytes`

This can still be CDN-backed by using a single CDN hostname (same origin) with path-based routing:

* `/assets/*` → static bucket/origin
* `/disks/*` → disk bytes origin (object storage, NGINX, CloudFront/S3 behavior, etc.)
* `/api/*` → application API origin

Benefits:

* No CORS preflight overhead.
* Works naturally with cross-origin isolation (`COOP/COEP`), because disk responses are same-origin.
* Cookie-based auth (“session cookies”) is straightforward.

#### Supported: cross-origin disk endpoints (CORS + COEP/CORP)

When disks must be served from a different origin (e.g., dedicated CDN domain):

* App: `https://app.example.com/`
* Disks: `https://disks.examplecdn.com/…`

Requirements for cross-origin disk origins:

* Correct **CORS** handling for:
  * `OPTIONS` preflights,
  * `Range` and optional `If-Range`, and
  * optional `Authorization` header.
* Responses compatible with the app’s **COEP** policy:
  * Same-origin is always OK.
  * Cross-origin resources must be CORS-enabled and/or send `Cross-Origin-Resource-Policy` (details in §6).

Operational guidance:

* Configure `Access-Control-Max-Age` for preflights to reduce request amplification (see §5).
* Prefer auth mechanisms that avoid cookies for cross-origin data plane (see §3) unless you intentionally want credentialed requests.

---

### Auth mechanisms and tradeoffs

All mechanisms below can implement a “disk access lease/capability”. The choice impacts:

* CORS preflight frequency,
* cacheability (browser + CDN),
* token leakage risk, and
* operational complexity.

#### Summary comparison

| Mechanism | Typical use | Cross-origin CORS complexity | CDN caching friendliness | Leakage risk | Notes |
|---|---|---:|---:|---:|---|
| Session cookies (same-origin) | Default for apps with login | Low (same-origin: none) | Medium (edge auth requires extra config) | Low | Best when disk endpoint is same-origin. |
| JWT bearer (`Authorization`) | API-style auth, non-browser clients | High (preflight; allow `Authorization`) | Low/Medium | Low | Avoid for CDN-cached byte ranges unless auth is validated at the edge. |
| Signed URL (token in query) | “Lease” URL from API | Medium (preflight for `Range`) | High | Medium | Works well with CDNs; keep tokens short-lived. |
| CloudFront signed URL | Signed URL, CloudFront-native | Medium | High | Medium | Similar to signed URL; uses CloudFront policies/keys. |
| CloudFront signed cookies | Cookie-based edge gating | High (credentialed CORS if cross-origin) | High | Low | Best cache hit rate for private content when using CloudFront; URL stays stable. |

#### Recommended default

**Default recommendation:** **Same-origin disk bytes endpoint + session cookies**, optionally combined with a **lease API** that returns a short-lived URL for the current session.

Rationale:

* Same-origin avoids CORS preflight overhead from `Range`.
* Session cookies keep credentials out of URLs and support standard web auth.
* A lease layer provides a clean abstraction for future sharing, for revocation-by-expiry, and for switching to signed URL/cookie delivery without changing client code.

#### When to use each mechanism

##### Session cookies (same-origin)
Use when:

* The disk bytes endpoint can be served from the **same origin** as the app.
* You want the simplest setup with minimal browser edge cases.

Notes:

* Prefer `SameSite=Lax` or `SameSite=Strict` cookies to reduce cross-site request abuse.
* Consider a separate “disk lease” cookie scoped to `/disks/*` with short TTL for tighter control.

##### JWT bearer (`Authorization: Bearer …`)
Use when:

* You need a unified auth story across web + non-web clients.
* You are not relying on shared CDN caching of private disk bytes (or you validate auth at the edge and strip/normalize the header for caching).

Tradeoffs:

* Cross-origin requests MUST preflight for `Authorization`.
* Forwarding `Authorization` to a CDN cache typically destroys cache hit rate (cache key varies per user/token) unless carefully configured.

##### Signed URLs (token in query)
Use when:

* You want the data plane to be **stateless** and/or served directly from object storage / CDN.
* You want **non-credentialed** fetches (`credentials: "omit"`) without cookies.

Tradeoffs:

* Tokens can leak via logs and `Referer` headers. Mitigations:
  * Use short expirations (minutes, not hours/days).
  * Scope the token to `{diskId, method=GET/HEAD, expiry}` at minimum.
  * Set the app `Referrer-Policy` to avoid leaking full URLs to other origins (e.g., `strict-origin-when-cross-origin` or `no-referrer`).

##### CloudFront signed URLs
Use when:

* You are on AWS/CloudFront and want CloudFront to enforce authorization at the edge.
* You want private images to remain CDN-cacheable without forwarding cookies/headers to origin.

##### CloudFront signed cookies
Use when:

* You need high cache hit rates for private images with stable URLs (no query tokens).
* You are willing to configure cookie distribution securely and handle credentialed requests if cross-origin.

Tradeoffs:

* If disks are on a different origin than the app, you MUST use credentialed CORS (`Access-Control-Allow-Credentials: true`) and cannot use `Access-Control-Allow-Origin: *`.
* `COEP: credentialless` is generally incompatible with cookie-based cross-origin disk fetches; prefer `COEP: require-corp` (see §6).

---

### HTTP protocol for disk bytes

#### Resource shape

The disk bytes endpoint is conceptually a static file:

* `GET /disks/{diskId}/bytes` returns bytes from offset `0..(size-1)`.
* `HEAD /disks/{diskId}/bytes` returns metadata (size, ETag, cache policy) without a body.

The resource **MUST be immutable for the lifetime of a lease**. If the underlying image changes, the URL and/or ETag MUST change (see ETag rules below).

#### `GET` semantics

##### No `Range` header

Servers MAY return `200 OK` with the entire disk image. Clients SHOULD NOT use full downloads for large images; prefer ranges.

##### Single-range requests (required)

Clients MUST use a single `Range` of the form:

* `Range: bytes=<start>-<end>` (inclusive indices, zero-based)

Servers MUST support this form and respond with:

* `206 Partial Content`
* `Content-Range: bytes <start>-<end>/<size>`
* `Accept-Ranges: bytes`
* `Content-Length: <end - start + 1>`

Servers MAY support `bytes=<start>-` (open-ended) but clients SHOULD NOT rely on it unless explicitly supported.

Multi-range requests (e.g., `bytes=0-1,4-5`) are **not** supported by this spec. Servers SHOULD reject them with `416 Range Not Satisfiable`.

#### `HEAD` semantics (required)

`HEAD` MUST be supported and MUST return:

* `200 OK`
* `Content-Length: <size>` (full resource size)
* `Accept-Ranges: bytes`
* `ETag` (see below)
* Appropriate cache headers (`Cache-Control`, etc.)

Clients SHOULD use `HEAD` to learn the disk size and current ETag before issuing `Range` reads.

#### Invalid ranges

If `Range` is syntactically invalid or not satisfiable, servers MUST return:

* `416 Range Not Satisfiable`
* `Content-Range: bytes */<size>`

#### ETag + `If-Range` (recommended)

Servers SHOULD return a strong `ETag` that changes whenever the disk image bytes change. Good candidates:

* a content hash (sha256), or
* a storage version identifier (S3 version ID), or
* `{diskId}:{generation}`.

Clients MAY use:

* `If-Range: "<etag>"` with a `Range` request.
* (Optional) `If-Range: <http-date>` when a `Last-Modified` value is available.

Behavior:

* If `If-Range` matches the current ETag, return `206` for the requested range.
* If it does not match (or the validator is invalid/weak), return `200 OK` with the full content (or `412 Precondition Failed` if your API prefers; if using `412`, document it and keep client code consistent).

Notes:

* The entity-tag form is strongly recommended for disk streaming (it allows safe resume even when filesystem/object-store mtimes are missing or coarse).
* The HTTP-date form compares at 1-second granularity (HTTP date resolution). Servers should compare `Last-Modified` and `If-Range` at second granularity to avoid false mismatches when the underlying store provides sub-second mtimes.

#### Compression must be disabled

Disk images are binary; servers and intermediaries MUST NOT apply compression transforms because it breaks deterministic byte offsets. Requirements:

* `Content-Encoding: identity`
* `Cache-Control` MUST include `no-transform`.
  * Aero's browser clients reject `206` responses without it (defence-in-depth against intermediary transforms).

#### Content type

Use:

* `Content-Type: application/octet-stream`
* `X-Content-Type-Options: nosniff` (recommended)

---

### CORS requirements (when cross-origin)

If the disk bytes endpoint is not same-origin with the app, the browser will perform CORS checks.

#### Why preflight happens

For Aero disk streaming, cross-origin `GET` requests typically include:

* `Range` (not CORS-safelisted) → triggers an `OPTIONS` preflight.
* Optionally `If-Range` (also not safelisted) → preflight.
* Optionally `Authorization` → preflight.
* Optionally `If-None-Match` / `If-Modified-Since` for conditional revalidation → preflight.

#### Required `OPTIONS` handling

The disk bytes endpoint MUST respond to `OPTIONS` (preflight) with at least:

* `Access-Control-Allow-Methods: GET, HEAD, OPTIONS`
* `Access-Control-Allow-Headers: Range, If-Range, If-None-Match, If-Modified-Since, Authorization` (include only those you actually use, but `Range` is required)
* `Access-Control-Max-Age: <seconds>` (recommended; see below)

Origin handling:

* **Non-credentialed mode (recommended for signed URLs):**
  * `Access-Control-Allow-Origin: *`
  * Do NOT set `Access-Control-Allow-Credentials`
* **Credentialed mode (cookies):**
  * `Access-Control-Allow-Origin: https://app.example.com` (MUST NOT be `*`)
  * `Access-Control-Allow-Credentials: true`
  * `Vary: Origin`

#### Required headers on `GET`/`HEAD` responses

For successful responses (`200`, `206`, and `416`), the disk bytes endpoint MUST include:

* `Access-Control-Allow-Origin: …` (either `*` or the specific origin)
* `Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding`

`Access-Control-Expose-Headers` is required so browser code can read:

* the returned range (`Content-Range`),
* the resource size (`Content-Length` on `HEAD` or `Content-Range` total size), and
* cache validators (`ETag`).
* and verify no compression transforms were applied (`Content-Encoding` should be absent or `identity`).

#### Preflight caching (`Access-Control-Max-Age`)

Preflights can be expensive with many small range reads. Set:

* `Access-Control-Max-Age: 600` (10 minutes) as a conservative default.
* Higher values (e.g., `86400`) MAY be used, but browsers may cap caching duration.

---

### COOP/COEP / cross-origin isolation requirements

To use `SharedArrayBuffer` (and thus WASM threads), the top-level app must be **cross-origin isolated**:

* `Cross-Origin-Opener-Policy: same-origin`
* `Cross-Origin-Embedder-Policy: require-corp` (recommended default)
  or `Cross-Origin-Embedder-Policy: credentialless` (alternative; see notes below)

#### Requirements for disk bytes responses

In a crossOriginIsolated app:

* Same-origin disk responses are always allowed.
* Cross-origin disk responses MUST be explicitly permitted. For `fetch()`-based byte reads, the practical requirement is:
  * the response is **CORS-enabled** (i.e., it passes CORS checks for the requesting origin), and
  * the response is not blocked by COEP.

For defence-in-depth and clarity, disk byte responses SHOULD also include:

* `Cross-Origin-Resource-Policy: same-site` when the disk origin is a subdomain of the app’s site (eTLD+1 matches), and you only intend same-site apps to read it.
* `Cross-Origin-Resource-Policy: cross-origin` when disks are intentionally served to multiple unrelated origins (e.g., a dedicated CDN domain used by multiple apps).

#### When to “rely on CORS” vs CORP

* CORS is required for JavaScript to read cross-origin bytes at all.
* CORP is primarily needed to satisfy `COEP: require-corp` for non-CORS subresource loads, but including it on disk responses is low-cost and prevents accidental breakage if the resource is consumed in other ways.

#### Notes on `COEP: credentialless`

`COEP: credentialless` can reduce some deployment friction by ensuring cross-origin subresource requests are made without cookies by default. However:

* Aero disk streaming still requires CORS for JS-readable responses.
* If you rely on **cookie-based** auth for a cross-origin disk endpoint (session cookies or signed cookies), prefer `COEP: require-corp` to avoid surprises; test carefully across browsers.

---

### CDN caching strategy

#### Public images

For publicly readable images (no per-user authorization), prefer long-lived immutable caching:

* `Cache-Control: public, max-age=31536000, immutable, no-transform`
* Stable `ETag` (content hash or generation ID)

#### Private images (safe defaults)

Default safe posture for private images:

* `Cache-Control: no-store, no-transform`

This prevents browsers and intermediary caches from storing private bytes where authorization is not enforced.

#### Private images with edge authorization (signed URL/cookie)

If authorization is validated at the edge (CDN) and the cached object is identical for all authorized users, you MAY use CDN caching for private images. Typical examples:

* Signed URL validated by CDN (token is part of cache key).
* Signed cookie validated by CDN (URL stable; CDN checks cookie before serving cached bytes).

In these cases, you can often use the same caching headers as public content **at the CDN layer**. Be explicit about where caching happens:

* At the **browser**: it is usually fine to keep `Cache-Control: no-store, no-transform` for private disks to avoid local persistence surprises.
* At the **CDN**: configure edge caching policies, and use origin headers appropriately for your CDN/provider.

#### Range caching and chunk alignment

Even with a CDN, `Range` caching can be inefficient if clients request arbitrary offsets. Clients SHOULD:

* Choose a fixed **chunk size** (default: **1 MiB** / 1,048,576 bytes).
* Align reads to chunk boundaries: request `bytes = floor(offset/chunk)*chunk … +chunk-1`.

This increases cache hit rates and reduces origin load because repeated reads map to identical range requests.

---

### Concrete examples

#### Example: `206 Partial Content` range response

Request:

```http
GET /disks/disk_123/bytes HTTP/1.1
Host: disks.examplecdn.com
Origin: https://app.example.com
Range: bytes=1048576-2097151
If-Range: "disk_123:gen_42"
```

Response:

```http
HTTP/1.1 206 Partial Content
Content-Type: application/octet-stream
Content-Encoding: identity
Cache-Control: no-transform
Accept-Ranges: bytes
Content-Range: bytes 1048576-2097151/42949672960
Content-Length: 1048576
ETag: "disk_123:gen_42"
Cross-Origin-Resource-Policy: same-site
Access-Control-Allow-Origin: https://app.example.com
Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding
Vary: Origin
```

#### Example: invalid range (`416`)

```http
HTTP/1.1 416 Range Not Satisfiable
Accept-Ranges: bytes
Content-Range: bytes */42949672960
Access-Control-Allow-Origin: https://app.example.com
Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding
Vary: Origin
```

#### Example: `OPTIONS` preflight (cross-origin + Range)

Preflight request:

```http
OPTIONS /disks/disk_123/bytes HTTP/1.1
Host: disks.examplecdn.com
Origin: https://app.example.com
Access-Control-Request-Method: GET
Access-Control-Request-Headers: range, if-range
```

Preflight response:

```http
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: https://app.example.com
Access-Control-Allow-Methods: GET, HEAD, OPTIONS
Access-Control-Allow-Headers: Range, If-Range, If-None-Match, If-Modified-Since, Authorization
Access-Control-Max-Age: 600
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

#### Example: lease API response schema

The control-plane API returns a short-lived lease that the client uses for subsequent `Range` reads.

```json
{
  "diskId": "disk_123",
  "url": "https://disks.examplecdn.com/disks/disk_123/bytes?cap=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expiresAt": "2026-01-10T12:34:56Z"
}
```

Notes:

* `cap` is an example capability token; the exact format is implementation-defined.
* Leases SHOULD be valid for minutes, not hours, and SHOULD be renewable by re-calling the lease API.

---

### Deployment profiles

- **AWS/CloudFront profile:** [`./deployment/cloudfront-disk-streaming.md`](../areas/storage.md)

## Disk Image Lifecycle and Access Control (Hosted Service)

### Overview

The open-source Aero emulator can read disk bytes from a URL using HTTP `Range` requests (see the streaming design in the storage subsystem). A **hosted Aero service** has additional requirements implied by [Legal & Licensing Considerations](../history/sprint-era-record.md):

- The service **must not provide Windows media**; users must upload/import their own.
- Uploaded Windows ISOs/disks must remain **private by default** and remain **usable over time**.
- The browser must be able to **stream** these private images securely.
- Depending on product tier, the system must support some form of **persistence/writeback** for changes made by the guest OS.

This document defines the disk image lifecycle, access control, and persistence strategies for a hosted Aero backend. It is written so a backend engineer can implement the flows without needing external references.

> Integration note: this document assumes the streaming/lease mechanism described in [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md). Leases/capabilities may be presented as signed URLs, cookies, and/or `Authorization` headers (see that doc for tradeoffs). This doc extends that model to cover uploads, ownership, sharing, and writeback.

Related documents (deployment/ops details):
- Streaming auth, CORS, COOP/COEP: [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md)
- CDN/object-store delivery: [Remote Disk Image Delivery (Object Store + CDN + HTTP Range)](../areas/storage.md)
- Range behavior + CDN limits (CloudFront): [HTTP Range + CDN Behavior](../areas/storage.md)
- CDN-friendly alternative to `Range`: [Chunked Disk Image Format](chunked-disk-image-format.md)
- Canonical disk/backend trait mapping: [Storage trait consolidation](../areas/storage.md)
- Concrete AWS setup (S3 + CloudFront signed URL/cookie + COOP/COEP): [deployment/cloudfront-disk-streaming.md](../areas/storage.md)
- Ops runbook for the bytes endpoint: [backend/disk-image-streaming-service.md](../areas/storage.md)
- Reference backend service (multipart upload + CloudFront signed cookies/URLs): [`services/image-gateway`](../decisions/README.md) (see also its ``openapi.yaml``)

---

### Terminology

- **Image**: A stored blob that can be streamed to the browser (ISO or block device content).
- **ISO image**: Optical media (installation media), always treated as read-only.
- **Disk image**: A hard-disk/SSD block device presented to the guest as a writable disk.
- **Base image**: Immutable blob used as the starting point (e.g., a clean Windows install, or an uploaded disk).
- **Delta / overlay**: Copy-on-write state layered over a base (local or remote).
- **Lease token**: Short-lived token granting a narrow capability (e.g., `disk:read` for a specific image).
- **User session**: Long-lived authentication (cookie/OAuth) used to call management APIs and request leases.
- **`imageId` / `diskId`**: stable identifier for an image. This doc uses `imageId` for management-plane APIs; the streaming auth spec uses `diskId` for the disk-bytes endpoint. They can be the same underlying ID.

---

### Typical hosted user flows (end-to-end)

#### A) Install Windows from a user-uploaded ISO (recommended default experience)

1) User uploads a Windows ISO (`kind=iso`, `visibility=private`).
2) User creates a new VM and provisions an **empty disk** of some size (e.g., 40 GiB).
   - The empty disk does not need an uploaded source file; the service can represent it as a “virtual zero” base with a declared `sizeBytes`.
3) When the VM starts, the browser requests short-lived `disk:read` leases for:
   - the ISO (CD-ROM) and
   - the disk base (empty disk base or uploaded base).
4) The browser streams data via `Range` reads from the disk bytes endpoint (e.g., `GET /disks/{diskId}/bytes`, sometimes routed as `/disk/{id}`; see [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md)).
5) Persistence:
   - **Strategy 1 (early default):** all writes go to an OPFS delta local to the browser.
   - **Strategy 2/3:** writes are persisted remotely under `disk:write`.

#### B) Import an existing VM disk (power user / migration)

1) User uploads/imports a disk image (`kind=disk`) in raw/qcow2/vhd.
2) Service converts to canonical raw (if needed) and marks it `ready`.
3) VM starts and streams the base disk read-only (plus delta, depending on writeback strategy).

---

### Image types: what users can upload vs what the service stores

#### Supported user-provided formats

The hosted service can accept multiple formats for user convenience, but should store **canonical formats** that are cheap to stream and easy for the browser disk layer to consume.

| User upload type | Typical extension(s) | Allowed? | Used as | Service canonical storage | Notes |
|---|---:|:---:|---|---|---|
| ISO 9660/UDF | `.iso` | ✅ | CD-ROM/DVD (install media) | **ISO blob as-is** | Read-only. Supports `Range` reads directly. |
| Raw disk | `.img`, `.raw` | ✅ | HDD/SSD | **Raw disk blob** (sector-addressable) | Best for streaming; simplest client. |
| QCOW2 | `.qcow2` | ✅ (import) | HDD/SSD | **Converted to raw disk blob** + optional “source” retention | QCOW2 is sparse/COW; converting avoids implementing QCOW2 in-browser. |
| VHD (fixed/dynamic) | `.vhd` | ✅ (import) | HDD/SSD | **Converted to raw disk blob** + optional “source” retention | VHD parsing/conversion best done server-side. |
| VHDX | `.vhdx` | ⚠️ Optional | HDD/SSD | Converted to raw disk blob | More complex; support later if needed. |

**Policy recommendation (early):**
- Accept **ISO + raw disk** initially.
- Add QCOW2/VHD import later with server-side conversion (async job).

#### What the service actually stores

For a hosted system, store images in object storage as **immutable objects** (even for “writable” disks; see writeback strategies). The one common exception is an **empty “zero disk” base**, which can be represented without pre-uploading gigabytes of zeros (e.g., by synthesizing zeros in the streaming endpoint, or by using a sparse/chunked representation).

Each image record should point to at least one underlying storage location:

- `canonicalObjectKey`: the canonical bytes object for the image (raw disk or ISO).
- Optional: `chunkedManifestObjectKey` (and `chunkedChunksPrefix`): precomputed chunked delivery artifacts (see [Chunked Disk Image Format](chunked-disk-image-format.md)).
- Optional: `sourceObjectKey`: the original uploaded file, retained for debugging/audit or re-conversion.

Delivery requirements (random access in the browser):
- **Range delivery:** the disk-bytes endpoint MUST support `GET` + `Range: bytes=...` with stable `Content-Length` (see [Disk Image Streaming](../areas/storage.md)).
- **Chunked delivery (optional):** alternatively, the service MAY serve read-only base images via a manifest + fixed-size chunk objects (see [Chunked Disk Image Format](chunked-disk-image-format.md)).
- In either case, avoid compression/transforms that break deterministic offsets (see the streaming/auth spec for required headers).

---

### Ownership, visibility, and sharing

#### Core fields

Each image record is owned by exactly one user:

- `id`: stable identifier (UUID/ULID).
- `ownerUserId`: user who created/imported the image.
- `kind`: `iso` or `disk`.
- `canonicalFormat`: `iso` or `raw` (even if the upload was qcow2/vhd).
- `sizeBytes`
- `createdAt`, `updatedAt`
- `state`: `uploading` → `processing` → `ready` (or `failed`, `deleted`)
- `visibility`: `private` \| `shared` \| `public`

#### Visibility meanings

- **`private` (default):** only the owner (and admins) can access.
- **`shared`:** access granted to specific principals (users and/or share links).
- **`public`:** readable by anyone via the Aero streaming endpoint (and optionally an unauthenticated “public read lease” flow); still no direct object-storage URLs are exposed.

#### Sharing mechanisms

Two supported sharing mechanisms can co-exist:

1) **Direct user sharing (ACL grants)**  
   An owner explicitly grants another user read/write access.

2) **Share links**  
   The service creates a random, revocable secret (a “link”) that acts as a principal.  
   This is useful for one-off sharing without creating accounts.

Recommended share link properties:
- Token value is **unguessable** (≥ 128 bits of entropy).
- Stored in DB as a **hash** (like password storage), so DB leaks do not expose active links.
- Links are **revocable** and can be time-limited.
- Link access should be **read-only by default** (read-write links are high risk).

#### Permissions matrix

Define actions in terms of image management and data plane access:

- `read`: stream bytes (`Range` reads) and view metadata.
- `write`: modify state (only applicable to delta/writeback strategies), update metadata like name/description, attach to VMs.
- `delete`: remove the image and all derived data (deltas, share links).
- `share`: create/revoke share grants and share links.
- `upload`: create/complete uploads (initial ingest only).

Roles/principals:
- **Owner**: `ownerUserId`
- **Shared user (read)**: explicit ACL entry
- **Shared user (write)**: explicit ACL entry
- **Share link holder**: anyone presenting a valid share-link token
- **Public/anonymous**: any caller (no auth)

| Principal | read | write | delete | share | upload |
|---|:---:|:---:|:---:|:---:|:---:|
| Owner | ✅ | ✅ | ✅ | ✅ | ✅ |
| Shared user (read) | ✅ | ❌ | ❌ | ❌ | ❌ |
| Shared user (write) | ✅ | ✅ | ❌ | ❌ | ❌ |
| Share link holder (default) | ✅ | ❌ | ❌ | ❌ | ❌ |
| Public/anonymous (`visibility=public`) | ✅ | ❌ | ❌ | ❌ | ❌ |

> Implementation tip: keep “management-plane” checks (list/update/delete/share/upload) on user session auth, and keep “data-plane” checks (stream bytes, write blocks) on short-lived leases.

---

### Suggested data model (relational)

This section is optional, but it is a good starting point for implementing the ownership/sharing semantics above.

#### `images`

One row per uploaded/imported image.

- `id` (PK)
- `owner_user_id` (indexed)
- `kind` (`iso` | `disk`)
- `canonical_format` (`iso` | `raw`)
- `canonical_object_key` (object storage key; immutable once `ready`)
- `source_object_key` (nullable; original upload)
- `size_bytes`
- `state` (`uploading` | `processing` | `ready` | `failed` | `deleted`)
- `visibility` (`private` | `shared` | `public`)
- `created_at`, `updated_at`

#### `image_acl`

Explicit grants for “shared” images.

- `id` (PK)
- `image_id` (FK → `images.id`, indexed)
- `principal_type` (`user` | `share_link`)
- `principal_id` (e.g., `userId` or `shareLinkId`)
- `permission` (`read` | `write`)
- `created_at`

> Policy suggestion: avoid “delete/share/upload” in ACL grants unless you have a strong use case; keep those owner-only.

#### `share_links`

Share links are principals; callers prove possession of the link token during the management-plane “redeem” flow or directly during lease issuance.

- `id` (PK)
- `image_id` (FK → `images.id`, indexed)
- `token_hash` (store a salted hash; never store the raw token)
- `permission` (`read` | `write`), but default to `read`
- `expires_at` (nullable)
- `revoked_at` (nullable)
- `created_at`

#### `uploads` (optional)

Track resumable uploads and multipart state.

- `id` (PK)
- `image_id` (FK → `images.id`, indexed)
- `provider` (`direct` | `s3` | `gcs` | …)
- `provider_upload_id` (multipart upload ID / resumable session ID)
- `state` (`active` | `completed` | `aborted`)
- `part_size_bytes`
- `created_at`, `expires_at`

#### `deltas` (Strategies 2/3)

Track remotely persisted writable state.

- `id` (PK)
- `owner_user_id` (indexed)
- `base_image_id` (FK → `images.id`, indexed)
- `block_size_bytes`
- `created_at`, `updated_at`

---

### Lifecycle states and transitions

#### Suggested state machine

```
create(metadata)
   │
   ▼
uploading  --(finalize upload)-->  processing  --(success)-->  ready
   │                                  │
   │                                  └--(failure)--> failed
   │
   └--(cancel/delete)--> deleted
```

Transitions:
- **uploading → processing:** the service has a complete object and starts validation/conversion.
- **processing → ready:** canonical object is available for streaming.
- **processing → failed:** keep diagnostics; allow retry by re-upload or re-convert.
- **ready → deleted:** delete canonical object and associated metadata (and deltas).

#### Validation and conversion (processing step)

Minimum recommended processing:
- Enforce size limits and per-user quotas.
- Detect format (do not trust filename/MIME type).
- For ISO: optionally sanity-check ISO headers/volume descriptors.
- For disk imports (qcow2/vhd): convert to canonical raw and record the resulting size/geometry.

Optional processing (product-dependent):
- Virus/malware scanning (be careful: scanning Windows images is non-trivial and may be expensive).
- Content hashing (SHA-256) for integrity checks and deduplication **within a single user**.

---

### Upload/import flows (browser → service)

Both flows start with the management plane (authenticated user session) creating an image record and obtaining authorization to upload bytes.

#### Common API shape (management plane)

1) **Create metadata**

`POST /v1/images`
```json
{
  "kind": "iso" | "disk",
  "displayName": "Windows 7 SP1 x64 ISO",
  "upload": {
    "filename": "Win7.iso",
    "sizeBytes": 3355443200
  }
}
```

Response:
```json
{
  "imageId": "img_...",
  "state": "uploading"
}
```

2) **Initiate upload** (choose A or B below)

3) **Finalize upload**

`POST /v1/images/{imageId}/upload:finalize`
```json
{
  "expectedSizeBytes": 3355443200,
  "optionalSha256": "..."
}
```

The server should then move the image to `processing` and later to `ready`.

> `disk:upload` scope is required for the upload initiation/finalization steps; uploading bytes themselves may be authorized by either a user session or a short-lived upload lease.

---

#### Approach A: direct upload to the Aero API (reference / self-hosted)

This approach is easiest to implement and deploy in a self-hosted environment, but pushes large data through your API servers.

##### Flow

1) `POST /v1/images/{imageId}/upload:begin`  
   Server returns an **upload token** (short-lived) and an endpoint to `PUT` to.

2) Browser uploads bytes to the Aero API:

`PUT /v1/images/{imageId}/content`
- `Authorization: Bearer <upload-lease>`
- `Content-Type: application/octet-stream`
- Use either:
  - **Single PUT** (small files)
  - **Chunked/resumable PUTs** (large files)

Resumable option (recommended if implemented):
- Client sends chunks with `Content-Range: bytes <start>-<end>/<total>`
- Server persists parts to temp storage and assembles on finalize.

3) `POST /v1/images/{imageId}/upload:finalize`

##### Pros / cons

Pros:
- No object-storage-specific client logic.
- Works without S3/GCS credentials or CORS complexity.

Cons:
- Expensive bandwidth/egress on API layer.
- Requires large request body handling, timeouts, buffering.
- Harder to make robust resumable uploads at tens of GB.

---

#### Approach B: direct-to-object-storage upload (signed URLs / multipart)

This approach keeps large uploads off your API servers while preserving strict access control.

##### Flow (multipart; S3/GCS-style)

1) `POST /v1/images/{imageId}/upload:begin`

Request:
```json
{
  "mode": "multipart",
  "partSizeBytes": 8388608
}
```

Response:
```json
{
  "uploadId": "upl_...",
  "partSizeBytes": 8388608,
  "parts": [
    { "partNumber": 1, "signedUrl": "https://storage..."},
    { "partNumber": 2, "signedUrl": "https://storage..."}
  ],
  "expiresAt": "2026-01-10T00:00:00Z"
}
```

2) Browser uploads each part with `PUT <signedUrl>` (or the provider’s multipart API).
   - Record each part’s ETag/checksum returned by the storage provider.

3) Browser finalizes the multipart upload with the Aero API:

`POST /v1/images/{imageId}/upload:complete`
```json
{
  "uploadId": "upl_...",
  "parts": [
    { "partNumber": 1, "etag": "\"...\"" },
    { "partNumber": 2, "etag": "\"...\"" }
  ]
}
```

4) Server completes the multipart upload server-side and transitions the image to `processing`.

##### Resumability notes

- Presigned URLs expire; resumability comes from keeping a stable `uploadId` and letting the client:
  - `GET /v1/images/{imageId}/upload` to list uploaded parts
  - `POST /v1/images/{imageId}/upload:refresh` to obtain new signed URLs for missing parts
- The browser should persist only **non-secret identifiers** (e.g., `imageId`, `uploadId`) if it needs to resume after reload.

##### Pros / cons

Pros:
- Scales to very large files; minimal load on API servers.
- Leverages storage provider durability and multipart semantics.

Cons:
- Requires CORS configuration on the bucket.
- Signed URLs are bearer secrets; must be handled carefully (see Security Controls).

---

### Secure streaming access (leases + Range reads)

#### Data plane endpoint

The browser streams image bytes via a `Range`-capable **disk bytes endpoint**, as specified in [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md):

- `GET /disks/{diskId}/bytes` (and `HEAD /disks/{diskId}/bytes`), supports `Range: bytes=...` (often routed as `GET /disk/{id}` in simplified deployments)

Private images MUST require a valid lease/capability (or equivalent authorization) on every request; public images may be cacheable and unauthenticated depending on policy.

Implementation detail: the disk bytes endpoint can be implemented as either:
- A same-origin service endpoint that proxies to object storage using backend credentials, or
- A CDN/object-storage URL gated by a short-lived signed URL/cookie lease.

The service SHOULD NOT hand out long-lived, permanent object-storage URLs for private images.

> Note: For some deployments, read-only base images may be delivered via a **chunked manifest + chunk objects** format to avoid `Range` and reduce CDN/cross-origin friction. In that mode, the same ownership/lease principles apply, but the data plane is `GET manifest.json` + `GET chunks/*.bin` instead of `Range` reads. See: [Chunked Disk Image Format](chunked-disk-image-format.md).

#### End-to-end diagram (auth → lease → streaming)

```
┌──────────────┐        ┌─────────────────────┐        ┌───────────────────────┐
│   Browser    │        │   Aero API (mgmt)   │        │  Aero Disk Endpoint   │
│ (user agent) │        │  user session auth  │        │ (data plane, Range)   │
└──────┬───────┘        └──────────┬──────────┘        └───────────┬───────────┘
       │                           │                               │
       │ 1) User login / session   │                               │
       │──────────────────────────▶│                               │
       │                           │                               │
       │ 2) Request lease:         │                               │
       │    POST /v1/leases        │                               │
       │    { diskId, scopes }     │                               │
       │──────────────────────────▶│                               │
       │                           │ 3) Issue short-lived lease    │
       │                           │    (e.g., signed URL or JWT)  │
       │◀──────────────────────────│                               │
       │                           │                               │
       │ 4) Stream bytes:          │                               │
       │    GET /disks/{id}/bytes  │                               │
       │    (aka /disk/{id})       │                               │
       │    Range: bytes=...       │                               │
       │    (auth via lease)       │                               │
       │──────────────────────────────────────────────────────────▶│
       │                           │                               │
       │ 5) 206 Partial Content    │                               │
       │◀──────────────────────────────────────────────────────────│
       │                           │                               │
```

Lease scope enforcement:
- The disk endpoint verifies:
  - lease validity (`exp`, signature, `aud`)
  - resource ID binding (e.g., `diskId`)
  - required scope (`disk:read` for reads; `disk:write` for writes where applicable)
  - optional rate limits / byte limits

---

### Persistence/writeback strategies (and auth implications)

Different products will choose different persistence models. The hosted service should support at least Strategy 1 initially, and be designed so Strategy 2/3 can be added without breaking the streaming interface.

#### Summary

| Strategy | What is remote? | What is local? | Cross-device resume | Server complexity | Required lease scopes |
|---|---|---|:---:|:---:|---|
| 1. Remote read-only + local OPFS delta (recommended early) | Base image (ISO/disk) | Delta/overlay (COW) | ❌ | Low | `disk:read` |
| 2. Remote base + remote delta | Base image + per-user delta | Optional cache | ✅ | Medium | `disk:read`, `disk:write` |
| 3. Fully remote read-write disk (block API) | Entire disk state | Optional cache | ✅ | High | `disk:read`, `disk:write` |

#### Strategy 1 (recommended early): remote base read-only + local OPFS COW delta

**Model**
- The service streams a **read-only base** (uploaded disk or a blank “template disk” created at VM creation time).
- The browser stores all writes in a **local copy-on-write delta** in OPFS (Origin Private File System).
- On boot:
  - Reads: check local delta first; fall back to remote base via `Range`.
  - Writes: go to delta only.

**Pros**
- Minimal backend complexity: no remote writes.
- Strong privacy: changes (including potentially sensitive user data) stay in the user’s browser storage.
- Great fit for “try it out” experiences.

**Cons**
- No cross-device resume.
- Clearing site data loses state.
- Browser storage quotas may be restrictive.

**Auth**
- Base streaming requires only `disk:read` leases.
- No `disk:write` is needed because the service never receives disk writes.

#### Strategy 2: remote base + remote delta (per-user, cross-device resume)

**Model**
- Keep the base immutable.
- Create a per-user (or per-VM) **delta image** stored remotely (object storage or DB-backed chunk store).
- Browser reads base + delta; writes are sent to the delta.

Recommended server-side delta representation:
- Fixed-size blocks (e.g., 1 MiB) addressed by `(deltaId, blockIndex)`.
- Store only written blocks (sparse).
- Optionally compact/merge blocks in the background.

**Pros**
- Cross-device resume.
- Easy backups and server-side retention policies.
- Allows sharing “machine state” without sharing base media.

**Cons**
- Requires a write API and careful quota enforcement.
- Concurrency/locking: decide whether multiple devices can write simultaneously (usually “single writer”).
- Potentially higher legal/compliance burden because the service now stores an installed Windows state (still user-provided, but it is persisted server-side).

**Auth**
- Reads: `disk:read` for base and delta.
- Writes: `disk:write` scoped to the delta (not necessarily the base).
- Recommended: issue **separate leases** for read vs write so a VM can be launched read-only.

#### Strategy 3: fully remote read-write disk (block API)

**Model**
- The disk is conceptually a remote block device.
- The browser uses a block read/write API (or `Range` for reads + separate write endpoint) to access the disk.
- Backend is the source of truth; client caching is an optimization only.

**Pros**
- True cross-device and server-side durability.
- Enables server-side snapshots, cloning, and collaborative scenarios (if desired).

**Cons**
- Highest complexity and cost: many small reads/writes, low-latency requirements.
- Requires caching, write coalescing, flush semantics, and conflict resolution.

**Auth**
- Requires `disk:read` + `disk:write` leases for the active VM session.
- Consider extra restrictions in the lease (rate limits, max bytes written) to reduce blast radius.

---

### Token and lease scopes

Beyond the baseline `disk:read`, the hosted service should define these scopes:

- `disk:read` — authorize streaming reads (`Range` GETs).
- `disk:write` — authorize writeback (delta writes or block writes).
- `disk:delete` — authorize deletion of an image and its derivatives.
- `disk:share` — authorize creating/revoking share grants/links.
- `disk:upload` — authorize upload initiation/finalization.

#### Least privilege model (recommended)

- **User session tokens** (cookies/OAuth):
  - Used for management-plane APIs: create/list/update/share/delete images; request leases.
  - Longer lifetime; higher privilege; never sent to the raw disk streaming endpoint from WASM workers if avoidable.

- **Leases** (short-lived bearer tokens):
  - Used for data-plane endpoints only: the disk bytes endpoint (`/disks/{diskId}/bytes`, sometimes routed as `/disk/{id}`) and any writeback endpoints.
  - Minted per image with explicit scopes and short TTL (minutes).
  - Presented as a signed URL, signed cookie, and/or `Authorization` header depending on deployment (see [Disk Image Streaming](../areas/storage.md)).
  - Renewed as needed (silent refresh) rather than being long-lived.

---

### Security controls (hosted service)

#### Storage security

- **Encryption at rest:** enable object storage server-side encryption (SSE). Prefer KMS-backed keys (SSE-KMS) for auditability.
- **Access isolation:** bucket policies/IAM should only allow the Aero backend role to read/write objects. No public ACLs for private images.
- **Access logging:** enable object access logs (and application logs) including `userId`, `imageId`, action (`read-range`, `upload-part`, `delete`), and bytes transferred.

#### Signing key rotation

- Lease tokens should include a `kid` header so signing keys can be rotated without downtime.
- Maintain multiple active verification keys; rotate on a schedule.
- For presigned URL generation credentials (S3 access keys / service accounts), rotate regularly and scope permissions to only the necessary bucket/prefix.

#### Quotas and abuse controls

- Enforce per-user quotas at **create** and **finalize** time:
  - max total bytes stored
  - max number of images
  - max size per image
- Rate-limit lease issuance and streaming requests (per user + per IP) to reduce scraping risk.

#### Avoid leaking signed URLs and tokens

Signed URLs (upload) and lease tokens (streaming) are bearer secrets.

Client guidance:
- Do not put long-lived user/session tokens in the **page URL** (no `?access_token=...`).
- If using signed URL leases for streaming (e.g., `?cap=...`), treat the full URL as a secret: do not persist it, do not log it, and keep expirations short (minutes).
- Do not persist signed URLs or leases in `localStorage` / `indexedDB`. Keep them in memory; re-issue when needed.
- Set `Referrer-Policy: no-referrer` (or at least `strict-origin`) on pages that may ever handle signed URLs to reduce accidental leakage via the `Referer` header. See also: [Security headers](../areas/security.md).

Server guidance:
- Return signed URLs only over HTTPS.
- Use tight expirations (minutes) and scope the URL to a single part/object.
- Set `Cache-Control: no-store` on responses that contain any secrets (leases, signed URLs).

---

### Appendix: Suggested API surface (v1)

Exact paths are implementation-defined; the goal is a clean separation between:

- **Management plane** (user session auth, slower, higher privilege): create images, upload initiation, share, delete, request leases.
- **Data plane** (lease/capability auth, high volume): `Range` reads and (optionally) writeback.

#### Image management (management plane)

Common endpoints (cookie/OAuth session auth; permission checks per the matrix above):

- `GET /v1/images` — list images visible to the caller.
- `GET /v1/images/{imageId}` — metadata (kind, size, owner, state, visibility).
- `PATCH /v1/images/{imageId}` — update metadata (e.g., display name, visibility).
- `DELETE /v1/images/{imageId}` — delete image + derived data (deltas, shares, share links).

#### Upload (management plane + upload lease)

- `POST /v1/images` — create an image record in `uploading`.
- `POST /v1/images/{imageId}/upload:begin` — return either:
  - an upload lease for direct API upload, or
  - multipart instructions + signed URLs for direct-to-object-storage upload.
- `PUT /v1/images/{imageId}/content` — (approach A) upload bytes to the API (optionally `Content-Range` resumable).
- `POST /v1/images/{imageId}/upload:complete` — (approach B) complete multipart upload by providing ETags.
- `POST /v1/images/{imageId}/upload:finalize` — transition to `processing` and start validation/conversion.

#### Sharing (management plane)

- `POST /v1/images/{imageId}/shares` — grant another user read or write (creates an ACL entry).
- `DELETE /v1/images/{imageId}/shares/{shareId}` — revoke an ACL entry.
- `POST /v1/images/{imageId}/share-links` — create a share link principal (store hashed token).
- `DELETE /v1/images/{imageId}/share-links/{linkId}` — revoke a share link.

#### Lease issuance (management plane)

The management plane mints short-lived leases/capabilities compatible with the disk-bytes endpoint described in [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md).

One possible shape:

`POST /v1/leases`
```json
{ "diskId": "img_...", "scopes": ["disk:read"], "ttlSeconds": 600 }
```

Response (signed URL example):
```json
{
  "diskId": "img_...",
  "url": "https://app.example.com/disks/img_.../bytes?cap=...",
  "expiresAt": "2026-01-10T12:34:56Z"
}
```

Response (bearer token example):
```json
{
  "diskId": "img_...",
  "authorization": "Bearer ...",
  "expiresAt": "2026-01-10T12:34:56Z"
}
```

#### Writeback endpoints (Strategies 2 and 3)

If the service supports remote persistence, keep write endpoints separate from the read-only base:

- **Strategy 2 (remote delta):**
  - Create a delta resource owned by the user (or per-VM), e.g. `deltaId`.
  - Writes go to the delta only; reads combine base + delta.
- **Strategy 3 (fully remote read-write):**
  - Treat the disk itself as a mutable resource, but still prefer immutable generations/snapshots under the hood.

A simple, CDN-agnostic write API uses fixed-size blocks:

- `PUT /v1/deltas/{deltaId}/blocks/{blockIndex}` — write an entire block (e.g., 1 MiB).
  - Requires a `disk:write` lease scoped to `{deltaId}` (or the effective disk resource).
  - Recommend `If-Match: "<generation>"` (or similar) to enforce single-writer semantics.
- `GET /v1/deltas/{deltaId}/blocks/{blockIndex}` — read a block (optional; many designs can serve reads via the normal disk-bytes endpoint).
- `POST /v1/deltas/{deltaId}:flush` — optional explicit flush/commit point (often a no-op if each block write is durable).

---

### Testable invariants (minimum)

- The service MUST NOT mint `disk:read`/streaming leases for images that are not in `ready`.
- The data plane MUST reject expired or scope-mismatched leases (typically `401`/`403`).
- A valid `Range` request MUST return `206 Partial Content` with a correct `Content-Range` and `Content-Length`.

## HTTP Range + CDN Behavior (CloudFront Focus)

### Overview

Aero streams large disk images to the browser using HTTP `Range` requests (`206 Partial Content`). Whether this is fast and cost-effective depends heavily on:

1. **Whether the CDN caches Range responses**, and
2. **Whether the CDN has a hard maximum cacheable response size** (a common gotcha for multi‑GB images).

This document turns the “will the CDN do the right thing?” uncertainty into concrete guidance:

- Clear yes/no answers for Amazon CloudFront (based on AWS documentation + common field observations).
- A safe **chunked-object strategy** to avoid running into CloudFront/CDN cacheable-size ceilings and to keep cache behavior predictable.
- Alternatives summary: Cloudflare (incl. R2) and a generic reverse proxy cache (Nginx).
- A checklist that operators can run with `curl` to validate their own deployment.

If you are looking for a **deployment runbook** (S3 + CloudFront + signed URLs/cookies, plus CORS/COEP/CORP headers), see:

- [`../areas/storage.md`](../areas/storage.md)

> **Assumptions / limitations**
>
> This repo does not contain AWS credentials or live infrastructure, so CloudFront behavior here is based on published AWS documentation and widely observed headers. Use the checklist below to validate the behavior in *your* distribution and region.

Related:
- [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md) (protocol + CORS/COEP requirements)
- [Remote Disk Image Delivery](../areas/storage.md) (object store + CDN deployment contract)
- [Chunked Disk Image Format](chunked-disk-image-format.md) (avoid `Range` entirely)

### Reference infrastructure in this repo

- Local development (MinIO + optional reverse proxy for edge/CORS emulation):
  - [`infra/local-object-store/README.md`](../decisions/README.md)
- AWS production reference (S3 + CloudFront tuned for Range + CORS):
  - [`infra/aws-s3-cloudfront-range/README.md`](../decisions/README.md)

---

### CloudFront: direct answers

#### Q1: Does CloudFront cache `206 Partial Content` per-range automatically? Do we need `Range` in the cache key?

- **Answer:** **Yes, CloudFront supports byte-range requests and caches content for them**, but **you should *not* include `Range` in the cache key**.
- **Why:** CloudFront treats Range requests as a first-class feature; caching is still keyed on the object URL (and whatever else you include in your cache key), and CloudFront can satisfy subsequent Range requests from the edge cache.
  - AWS docs: on a `Range GET`, CloudFront checks the edge cache; if the requested range is missing, it forwards the request to the origin and **“caches it for future requests.”**

**Recommendation for Aero:** Keep the cache key minimal (path + version). Do *not* vary/cache-key on `Range`; doing so will fragment the cache and destroy hit ratio.

**Important operational note:** when validating `206` caching, make sure you’re comparing requests handled by the **same edge cache node**. CloudFront POPs often return multiple IPs and the `Via:` value can change between connections; caches are not necessarily shared across those nodes.

- If you run `curl` twice as separate processes, you can hit different cache nodes and see `X-Cache: Miss from cloudfront` both times even though caching is working.
- Prefer to either:
  - reuse the same connection (example below), or
  - pin to one CloudFront edge IP with `curl --resolve`.

Example (single `curl` process, same connection → you should typically see `Miss` then `Hit`):

```bash
URL='https://d3njjcbhbojbot.cloudfront.net/webapps/r2-builds/front-page/en.app.1cc5a48243bc23fab536.js?cachebust=aero-range-test'
curl -fsS -D - -o /dev/null -H 'Range: bytes=0-1023' "$URL" \
  --next -fsS -D - -o /dev/null -H 'Range: bytes=0-1023' "$URL" \
  | grep -iE '^(x-cache|age|via|x-amz-cf-pop|content-range):'
```

#### Q2: Can CloudFront serve/caches Range requests for objects larger than CloudFront’s maximum cacheable file size?

- **Answer:** **Yes, as long as you only retrieve the object in parts using Range requests.**
- **CloudFront limit:** CloudFront’s **maximum cacheable file size per HTTP GET response is 50&nbsp;GB** (historically 20&nbsp;GB; see current CloudFront limits). With caching enabled, a **non-Range `GET`** for an object larger than this can fail.
- **Why Range helps:** A `Range` response’s `Content-Length` is only the requested slice. AWS documents that CloudFront will **cache the requested range** and can serve subsequent requests for the same range from the edge cache.
- **Mitigation for very large images:** If your disk image might exceed 50&nbsp;GB, either:
  - (A) enforce **range-only** access (no full `GET`; avoid `HEAD`—use `Range: bytes=0-0` to learn total size via `Content-Range`), or
  - (B) publish the image as **chunk objects** (details below) for simpler operational behavior and portability across CDNs.

#### Q3: What about `Cache-Control: immutable` and long TTLs for `206`?

- **Answer:** CloudFront caching is governed by **TTL (`max-age` / `s-maxage` / `Expires`) and CloudFront cache policy caps**, not by the `immutable` token.
- **Practical effect:** You can (and should) use long-lived immutable caching headers on disk image chunks. Just ensure URLs are versioned (hash in the path) so you never need to “update in place.”

#### Q4: What headers indicate cache hits/misses for `206`?

CloudFront typically returns the same debugging headers for `206` as for `200`:

- `X-Cache`: `Miss from cloudfront`, `Hit from cloudfront`, `RefreshHit from cloudfront`, `Error from cloudfront`
- `Age`: present/increasing on cache hits (per edge location)
- `Via`, `X-Amz-Cf-Pop`, `X-Amz-Cf-Id`: useful for correlating POP behavior

**Spot-check (public CloudFront distribution, Jan 2026):**

```bash
curl -fsS -D - -o /dev/null -H 'Range: bytes=0-1023' \
  'https://d3njjcbhbojbot.cloudfront.net/favicon.ico?cachebust=aero-range-test' \
  | grep -iE '^(http/|x-cache|age|content-range):'
```

On first request: `x-cache: Miss from cloudfront` (often `age` absent). Repeating the same request typically returns `x-cache: Hit from cloudfront` with a small `age: <n>`.

---

### CloudFront: what to configure for Aero

#### Origin must support byte ranges correctly

The origin must return correct `206` responses when given `Range: bytes=...`, including:

- `Accept-Ranges: bytes` (strongly recommended)
- `Content-Range: bytes START-END/TOTAL`
- A stable validator (`ETag` is ideal)
- A concrete length (`Content-Length`) for the returned range

**CloudFront gotcha:** AWS documents that if the viewer makes a `Range GET` request and the origin responds with `Transfer-Encoding: chunked`, CloudFront can return the **entire object** instead of the requested range. For disk images, ensure your origin always sets `Content-Length` on `206` responses and does not use chunked transfer encoding.

For maximum CDN compatibility, clients should send **a single byte range per request** (avoid multipart `Range: bytes=a-b,c-d`).

S3 origins satisfy this naturally for normal objects.

#### Cache policy (cache key + TTL)

For a disk image or chunk object that never changes once published:

- **Cache key:** path only (and optionally a *version* query param if you can’t put the version in the path).
  - Avoid including headers/cookies in the cache key.
  - Do **not** include `Range` in the cache key.
- **TTL:** set for long-lived caching:
  - Prefer `Cache-Control: public, max-age=31536000, immutable, no-transform`
  - Ensure CloudFront cache policy **Maximum TTL** is >= your origin’s `max-age` or CloudFront will cap it.

**Why this matters for Aero:** random-access reads cause many repeated small Range reads. Any extra cache-key variance (cookies, headers) will cause near-0% hit ratio.

#### CORS (browser requirement)

`Range` is not a CORS-safelisted request header, so browser fetches will trigger a preflight.

Ensure:

- `Access-Control-Allow-Methods: GET, HEAD, OPTIONS`
- `Access-Control-Allow-Headers: Range, If-Range, If-None-Match, If-Modified-Since, Authorization` (include only what you use)
- `Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding`
  - Note: `Last-Modified` is a CORS-safelisted response header and is exposed to JS by default (no `Expose-Headers` needed).

If using S3 as origin, configure the bucket CORS rules accordingly. If using a custom origin, ensure OPTIONS is handled.

---

### CloudFront limits: max cacheable size (50&nbsp;GB) and what to do about it

CloudFront has a documented **maximum cacheable file size per HTTP GET response of 50&nbsp;GB**.

- If the object is **≤ 50&nbsp;GB**, CloudFront can cache a normal `200 OK` response and then satisfy subsequent `Range` requests from the cached object (best-case behavior).
- If the object is **> 50&nbsp;GB** and caching is enabled, a non-Range `GET` can fail. However, AWS documents that you can still use CloudFront by retrieving the object with **multiple Range GETs**, each returning a response `< 50&nbsp;GB`; CloudFront caches each requested part for future requests.

For Aero disk streaming (many small random-access reads), depending on Range-caching behavior at your POP, relying on per-Range caching can create many cache entries. A more predictable approach is to publish the image as fixed-size **chunk objects** (or adopt the no-Range format in [`18-chunked-disk-image-format.md`](chunked-disk-image-format.md)).

#### Recommended mitigation (portable): store disk images as chunk objects

Instead of one URL for the entire disk image:

```
/images/<image_id>/disk.img
```

publish a directory of fixed-size chunks:

```
/images/<image_id>/chunks/000000.bin
/images/<image_id>/chunks/000001.bin
...
```

And a small manifest:

```
/images/<image_id>/manifest.json
```

##### Chunk size recommendations

Choose a chunk size that:

- Is **well under** CloudFront’s max cacheable response size (50&nbsp;GB) and any object-store limits.
- Is not so large that a single cache miss is painful.

Practical ranges (chunk objects with optional intra-chunk `Range`):

- **64 MiB – 512 MiB** per chunk (keeps object counts low while still allowing `Range` reads within each chunk object).
- If you want to avoid `Range` preflights entirely (plain `GET` of whole chunk objects), choose a chunk size closer to your client fetch unit. For Aero’s no-Range chunked disk format, the recommended default is **4&nbsp;MiB** (see [`18-chunked-disk-image-format.md`](chunked-disk-image-format.md)).

##### Client mapping: offset → chunk + intra-chunk range

Given:

- `chunkSize` (bytes)
- `offset` (bytes into the virtual disk)
- `length` (bytes to read)

Compute:

```
chunkIndexStart = floor(offset / chunkSize)
chunkIndexEnd   = floor((offset + length - 1) / chunkSize)
```

For each chunk `i` in `[chunkIndexStart, chunkIndexEnd]`:

```
chunkBase = i * chunkSize
rangeStartInChunk = max(offset, chunkBase) - chunkBase
rangeEndInChunk   = min(offset + length, chunkBase + chunkSize) - 1 - chunkBase

GET /images/<image_id>/chunks/<i>.bin
Range: bytes=<rangeStartInChunk>-<rangeEndInChunk>
```

This strategy:

- Works on CDNs with cacheable-size limits.
- Lets the CDN cache each chunk independently.
- Makes origin load and cache hit ratios predictable.

---

### Alternative: Cloudflare CDN (+ R2 / Workers)

Cloudflare is a reasonable option for hosting Aero disk bytes, especially when paired with **Cloudflare R2**. The key gotchas are:

1. **Edge cacheability has strict size limits**, and
2. Range behavior depends on whether the origin response is suitable for ranged delivery.

#### Range / `206` support (what to expect)

Cloudflare documents that clients can send range requests using the `Range` header, and:

- If the **origin response includes `Content-Length`**, Cloudflare will return the requested range with **HTTP `206 Partial Content`**.
- If the **origin response does not include `Content-Length`**, Cloudflare will return the full content with **HTTP `200`**.

This means Cloudflare can silently “fall back” to full-object responses if your origin uses chunked transfer encoding or otherwise omits `Content-Length`.

#### Observable cache headers

Typical Cloudflare cache-debugging headers:

- `CF-Cache-Status`: `HIT`, `MISS`, `BYPASS`, etc.
- `Age`: on cache hits (similar semantics to other CDNs)

**Spot-check (public Cloudflare, Jan 2026):**

```bash
curl -fsS -D - -o /dev/null -H 'Range: bytes=0-10' \
  'https://www.cloudflare.com/ips-v4' \
  | grep -iE '^(http/|cf-cache-status|age|content-range):'
```

#### Hard limits: cacheable file size

Cloudflare documents the following **cacheable file size limits**:

- **Free/Pro/Business:** 512&nbsp;MB
- **Enterprise:** 5&nbsp;GB by default (higher on request)

For Aero disk images (20–40&nbsp;GB+), this means **chunk objects are mandatory** if you want any edge caching.

#### Using R2 as origin

Cloudflare R2 notes relevant to Aero:

- R2’s “public development URL” (`r2.dev`) is intended for non-production and does **not** provide features like caching/WAF/bot management; Cloudflare recommends using a **custom domain** for production.
- If you serve disk bytes via **Workers + R2**, the R2 Workers API supports **ranged reads** (getting `{ offset, length }` slices). You can implement HTTP `Range` by parsing the viewer `Range` header, calling R2 with a range, and returning a `206` response with `Content-Range`/`Content-Length`.

**Recommendation for Aero on Cloudflare:**

- Prefer the no-Range chunk format in [`18-chunked-disk-image-format.md`](chunked-disk-image-format.md), or fixed-size chunk objects with optional intra-chunk `Range`.
- Keep chunk size **≤ 512&nbsp;MB** unless you are on Enterprise and have verified a higher cacheable size limit.
- Ensure your origin responses include `Content-Length` (avoid `Transfer-Encoding: chunked`).

---

### Alternative: Nginx reverse proxy cache (`proxy_cache`) + Range

If you run your own CDN-like edge or regional caching layer with Nginx:

- Nginx can serve Range requests for static files easily.
- For **proxy caching large objects with Range**, the robust approach is to use the **`slice`** module so the cache stores fixed-size slices rather than arbitrary user-requested ranges.

#### Recommended Nginx config pattern (slice caching)

```nginx
# Cache storage (size/tuning are examples)
proxy_cache_path /var/cache/nginx/aero
  levels=1:2
  keys_zone=aero:1g
  max_size=500g
  inactive=30d
  use_temp_path=off;

server {
  location /images/ {
    proxy_pass https://origin.example.com;

    # Slice into fixed 1 MiB subrequests.
    # This normalizes all arbitrary viewer Range requests into stable cache keys.
    slice 1m;
    proxy_set_header Range $slice_range;

    proxy_cache aero;
    proxy_cache_lock on;

    # Cache both 200 and 206.
    proxy_cache_valid 200 206 30d;

    # Include slice range in the cache key.
    proxy_cache_key "$scheme://$host$uri$is_args$args|$slice_range";

    add_header X-Cache-Status $upstream_cache_status always;
  }
}
```

**Why `slice` matters:** without slicing, every distinct `Range: bytes=a-b` can become a distinct cache entry, which explodes cache cardinality and destroys hit ratio for random access patterns.

---

### Operator validation checklist (copy/paste)

Run these tests against your **CDN URL** (not the origin), ideally from two different machines/regions:

#### Basic Range correctness

```bash
URL="https://YOUR_DOMAIN/images/IMAGE_ID/chunks/000000.bin"
curl -fsS -D - -o /dev/null -H 'Range: bytes=0-1023' "$URL"
```

Confirm:

- `HTTP/* 206`
- `Content-Range: bytes 0-1023/<total>`
- `Accept-Ranges: bytes` (often present on `HEAD`/`200`; may not appear on `206` for some origins)

#### Cache hit behavior for `206`

Repeat the same request twice.

> Tip: if you run two separate `curl` commands, CloudFront may route you to different edge cache nodes. For the cleanest signal, reuse the same connection with `--next`:

```bash
curl -fsS -D - -o /dev/null -H 'Range: bytes=0-1023' "$URL" \
  --next -fsS -D - -o /dev/null -H 'Range: bytes=0-1023' "$URL" \
  | grep -iE '^(x-cache|age|via|x-amz-cf-pop):'
```

Expect on CloudFront:

- First request: `X-Cache: Miss from cloudfront` (often `Age` absent/0)
- Second request: `X-Cache: Hit from cloudfront` and `Age: <n>`

If the second request is still a `Miss`, run a full `GET` and try again:

```bash
curl -fsS -D - -o /dev/null "$URL" | grep -iE '^(x-cache|age|via|x-amz-cf-pop):'
curl -fsS -D - -o /dev/null -H 'Range: bytes=0-1023' "$URL" | grep -iE '^(x-cache|age|via|x-amz-cf-pop):'
```

If `GET` becomes a hit but `Range` stays a miss, assume `206` caching is not working for your workload and use chunk objects / no-Range delivery.

#### Different range behavior

```bash
curl -fsS -D - -o /dev/null -H 'Range: bytes=1048576-1049599' "$URL" | grep -iE '^(http/|x-cache|age|content-range):'
```

This should still be `206`, with an appropriate `Content-Range`, and after repeating it should become a hit.

#### Verify the CDN isn’t “cheating” by fetching full objects on a Range miss

Enable origin access logs (or per-request logging) and inspect what the CDN requested from the origin when the viewer asked for `Range: bytes=0-0`.

- CloudFront may request a **larger range than the viewer asked for** (AWS documents this as an optimization). This is usually fine; you’re checking for “downloaded the whole object” vs “downloaded a bounded range.”
- If the origin sees `Range: bytes=0-0`, the CDN is fetching only the requested bytes (good).
- If the origin sees a full `GET` with a large transfer size, the CDN is filling cache by downloading the entire object on first Range miss (still works, but makes large single-objects very expensive).

#### Object size ceiling check (CloudFront-specific)

Publish (or identify) an object whose total size is **> 50 GB** and attempt a tiny range:

```bash
curl -v -o /dev/null -H 'Range: bytes=0-0' "https://YOUR_CLOUDFRONT_DOMAIN/path/to/oversize-object"
```

If the `Range` request fails, you likely need chunk objects (or a different delivery mechanism).

Also test how a non-Range request behaves (avoid downloading the full body; use `HEAD`):

```bash
curl -v -I "https://YOUR_CLOUDFRONT_DOMAIN/path/to/oversize-object"
```

If `HEAD` fails for oversized objects in your configuration, avoid `HEAD` in clients and use `Range: bytes=0-0` to discover total size via `Content-Range`.

---

### References (vendor docs)

- AWS CloudFront Developer Guide (Range GETs): https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/RangeGETs.html
- AWS CloudFront quotas/limits (max cacheable file size per GET response): https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/cloudfront-limits.html
- Cloudflare default cache behavior (range requests + cacheable size limits): https://developers.cloudflare.com/cache/concepts/default-cache-behavior/
- Cloudflare R2 public buckets (r2.dev limitations): https://developers.cloudflare.com/r2/buckets/public-buckets/
- Cloudflare R2 Workers API (ranged reads): https://developers.cloudflare.com/r2/api/workers/workers-api-reference/
- RFC 9110 (HTTP Semantics, Range): https://www.rfc-editor.org/rfc/rfc9110.html
- RFC 9111 (HTTP Caching, partial responses): https://www.rfc-editor.org/rfc/rfc9111.html

## Disk Image Streaming Service (Runbook)

This document covers operational concerns for the **disk image streaming service**: how it is used by the browser client, the required HTTP/CORS headers for `Range` support, how to deploy it behind a reverse proxy, and how to troubleshoot the most common failures (especially around cross-origin isolation).

### Purpose

The disk image streaming service exists to make multi‑GB disk images usable in the browser without downloading them up front.

### Reference implementation in this repo

This repository includes a minimal reference service at ``services/image-gateway/`` that implements:

- S3 multipart uploads (presigned `UploadPart` URLs)
- immutable/versioned object keys for stable CDN URLs
- CloudFront signed cookies (preferred) or signed URLs for viewer authorization
- a local-dev `Range` proxy fallback endpoint

The exact paths differ from the “planned” section below (which focuses on a simplified pure-streaming service),
but the HTTP semantics and header requirements are the same.

On the client side, the storage subsystem uses `StreamingDisk` (see [05 - Storage Subsystem](../areas/storage.md)) to:

1. Read sectors from a *virtual* disk.
2. Lazily fetch missing byte ranges from a remote image over HTTP `Range`.
3. Cache fetched chunks locally (e.g. OPFS + sparse format).

Implementation note: the canonical Rust `StreamingDisk` implementation lives in
`crates/aero-storage` (`aero_storage::StreamingDisk`). For the repo-wide canonical disk/backend
trait mapping, see: [`20-storage-trait-consolidation.md`](../areas/storage.md).

This allows the emulator to boot quickly and only download the parts of the OS/image that are actually accessed.

For the hosted-service model (user uploads, ownership/visibility, lease scopes, and writeback options), see: [Disk Image Lifecycle and Access Control](../areas/storage.md).

For the normative protocol/auth/CORS contract of the disk bytes endpoint, see: [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md).

### How it interacts with `StreamingDisk`

`StreamingDisk` issues HTTP requests like:

```http
GET /disks/disk_123/bytes HTTP/1.1
Host: disks.examplecdn.com
Range: bytes=1048576-2097151
Origin: https://app.example.com
```

The streaming service must:

- Return **`206 Partial Content`** for satisfiable ranges.
- Include **`Content-Range`** and **`Accept-Ranges`** headers.
- Support **CORS preflight** (because `Range` is not a CORS-safelisted header).
- Expose required response headers to the browser (so `StreamingDisk` can discover total size / validate responses).

### Endpoint summary

These endpoints are recommended for operability. Exact paths may vary, but the semantics should be consistent.

#### Health / readiness / metrics

| Endpoint | Method | Purpose | Notes |
|---|---:|---|---|
| `/healthz` | GET | Liveness | “Process is up”; should not check external deps. |
| `/readyz` | GET | Readiness | “Can serve images”; should check storage backend connectivity (filesystem/S3/etc). |
| `/metrics` | GET | Prometheus metrics | Protect this endpoint (network policy or auth). |

#### Control-plane: image catalog + metadata (implemented)

| Endpoint | Method | Purpose | Notes |
|---|---:|---|---|
| `/v1/images` | GET | List available disk images (JSON) | Includes size/etag/last_modified; response uses `Cache-Control: no-cache`. |
| `/v1/images/{image_id}/meta` | GET | Fetch metadata for one image (JSON) | Same fields as the list response. |

Implementation note:

- Treat `image_id` as an opaque identifier and keep it **bounded** for operability and DoS safety
  (the reference `aero-storage-server` implementation enforces **`[A-Za-z0-9._-]{1,128}`**
  and disallows `.` / `..`).

Metadata response fields:

- `id`, `name`, `description` (optional)
- `size_bytes`, `etag`, `last_modified`, `content_type`
- `recommended_chunk_size_bytes` (optional)
- `public` (boolean)

#### Image streaming (planned)

| Endpoint | Method | Purpose | Notes |
|---|---:|---|---|
| `/disks/{disk_id}/bytes` | GET | Stream raw disk bytes | Must support `Range`. (Some deployments route this as `/disk/{id}` or `/v1/images/{id}`.) |
| `/disks/{disk_id}/bytes` | HEAD | Fetch metadata headers only | Useful for verifying `Content-Length`/`ETag`. |
| `/disks/{disk_id}/bytes` | OPTIONS | CORS preflight | Must include `Access-Control-*` headers. |
| `/v1/images/{image_id}/meta` | GET | JSON metadata (control-plane) | Image discovery + `size_bytes`/`etag` prior to `Range` reads. |

### Configuration

The service should be configurable enough to support multiple storage backends and safe production deployments. At minimum, plan for configuration knobs in these areas:

- **Network**
  - Listen address/port (and optional base path/prefix if mounted behind a proxy).
  - TLS termination strategy (direct TLS vs behind a reverse proxy).
- **Image store**
  - Filesystem: root directory containing image files.
  - Object storage: bucket name + prefix, credentials, and region/endpoint.
  - Optional integrity checks (checksums, signed manifests).
- **Image identification**
  - Map a stable `image_id` (e.g. `win7-sp1-x64`) to an underlying object key/path.
  - Avoid exposing raw filesystem paths to callers (see security guidance).
  - Keep `image_id` values bounded and restricted to safe characters to avoid
    path traversal and to prevent log/metrics amplification attacks.
- **CORS**
  - Allowlist of app origins allowed to fetch images (or `*` only for non-credentialed public images).
  - Whether to include `Access-Control-Allow-Credentials`.
  - `Access-Control-Max-Age` for caching preflight responses.
- **Range policy**
  - Maximum allowed range length (protects against huge single requests).
  - Maximum concurrent requests per client/token.
  - Whether to support only single-range requests (recommended).
- **AuthN/AuthZ**
  - Signed URL validation, bearer token/JWT verification, or mTLS requirements.
  - Per-image authorization policy (who can fetch which `image_id`).
- **Observability**
  - Access logging (include method/path/status and `Range`).
  - Metrics (bytes served, request counts, 206/416 rates, error rates).

### HTTP requirements for browser compatibility

#### Range support (the “206 contract”)

The service must implement **single-range** requests with the `bytes` unit.

Minimum behavior:

- **Requests:**
  - `Range: bytes=start-end` (inclusive, 0-indexed)
  - `Range: bytes=start-` (open-ended) is strongly recommended.
- **Responses (success):**
  - Status: `206 Partial Content`
  - `Accept-Ranges: bytes`
  - `Content-Range: bytes start-end/total_size`
  - `Content-Length: end-start+1`
- **Responses (unsatisfiable):**
  - Status: `416 Range Not Satisfiable`
  - `Content-Range: bytes */total_size`

Notes:

- Avoid content transformations. **Disable compression** (see below). Byte ranges are defined over the *wire representation*; a `Content-Encoding: gzip` response makes “disk byte offsets” meaningless.
- If using caching/CDNs, ensure `Range` is forwarded and not normalized away.

#### HTTP caching (ETag / Last-Modified / Cache-Control)

To make `StreamingDisk` range reads cache-friendly (browser cache + CDN) while remaining correct:

- **Always include validators when available**
  - `ETag` (strongly recommended)
  - `Last-Modified` (when the underlying image has a meaningful mtime)
  - ETag values should be quoted HTTP entity-tags and use visible ASCII so clients can reliably
    round-trip them through `If-None-Match` / `If-Range`.
- **Return `304 Not Modified`** for conditional `GET`/`HEAD` where applicable.
- **Set explicit `Cache-Control`**
  - Metadata endpoints should be revalidated (`no-cache`) rather than cached blindly.
  - Raw bytes can be cached aggressively for public/immutable images, but must not be cached for
    authenticated/private responses.
- Ensure cache-aware responses include the correct `Vary` headers (at minimum `Vary: Origin` if the
  service varies CORS responses by `Origin`).

Recommended policy:

- **Metadata** (`/v1/images`, `/v1/images/{image_id}/meta`)
  - `Cache-Control: no-cache`
  - Always send `ETag`
  - Support `If-None-Match` on `GET`/`HEAD` (return `304` when matched)
- **Data** (`/disks/{disk_id}/bytes`)
  - Send `ETag` and `Last-Modified` when available
  - `Accept-Ranges: bytes`
  - Public images: `Cache-Control: public, max-age=<n>, no-transform` (max-age configurable)
  - Authenticated/private images: `Cache-Control: private, no-store, no-transform`
  - `HEAD` should support `If-None-Match` / `If-Modified-Since` and return `304` when matched.
  - If implementing `If-Range`, follow RFC 9110:
    - `If-Range` may be an entity-tag (preferred) **or** an HTTP-date.
    - If the validator doesn't match the current image version, ignore the `Range` header and return a full `200`
      response (to avoid mixed-version bytes).
    - When evaluating the HTTP-date form, compare at 1-second granularity (HTTP date resolution) to avoid false
      mismatches with sub-second mtimes.

#### Required CORS headers (including `Range`)

Because browsers preflight cross-origin `Range` requests, you must support `OPTIONS` and include the correct headers on both the preflight and the actual response.

Recommended **preflight response** headers:

```http
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: https://app.example.com
Access-Control-Allow-Methods: GET, HEAD, OPTIONS
Access-Control-Allow-Headers: Range, If-Range, If-None-Match, If-Modified-Since, Authorization
Access-Control-Max-Age: 600
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

Recommended **GET/HEAD response** headers:

```http
Access-Control-Allow-Origin: https://app.example.com
Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding
Vary: Origin
Accept-Ranges: bytes
```

Important details:

- `Content-Range`, `Accept-Ranges`, and `Content-Length` are **not** CORS-safelisted response headers. If they are not listed in `Access-Control-Expose-Headers`, the fetch may succeed but the browser will hide these headers from JS.
- Request headers like `If-None-Match` and `If-Modified-Since` are **not** CORS-safelisted. If you use conditional requests (for metadata revalidation or full-body reads), ensure your preflight response allows them.
- If you need cookies or other credentials, you must **not** use `Access-Control-Allow-Origin: *`; instead echo a specific origin and also include `Access-Control-Allow-Credentials: true`.

#### Cross-origin isolation: COOP/COEP vs CORS/CORP

To use `SharedArrayBuffer` (WASM threads), the **app origin** must be cross-origin isolated. This is usually done with:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp` (or `credentialless`)

However, cross-origin isolation changes how the browser treats **cross-origin subresources**, including disk image fetches.

Recommended deployment model:

- **App origin** (HTML/JS/WASM):
  - Serve the emulator UI here.
  - Set COOP/COEP on the HTML (and ensure the document is eligible for cross-origin isolation).
- **Image origin** (disk images):
  - Serve raw images here.
  - Enable CORS for the app origin.
  - Consider setting `Cross-Origin-Resource-Policy` to explicitly allow the app to load the resource:
    - `Cross-Origin-Resource-Policy: same-site` if `app.example.com` and `images.example.com` share the same “site”.
    - `Cross-Origin-Resource-Policy: cross-origin` if the image host must be embeddable from multiple sites.

If you see COEP-related console errors, it almost always means the image origin is missing CORS/CORP (see troubleshooting).

### Recommended reverse proxy settings

Even if the service itself is correct, reverse proxies and CDNs commonly break `Range` in subtle ways.

General recommendations:

- **Prefer HTTP/2** end-to-end (browser → edge → service) for multiplexing many small range requests.
- **Increase timeouts** on the image route (range reads are small but can be numerous).
- **Disable compression** on image routes (to preserve byte offsets).
- Avoid buffering huge responses in memory; pass through streaming responses.

#### Example: NGINX (proxying to the service)

```nginx
# Disk images are large and use byte ranges; avoid transformations.
location /disks/ {
  proxy_pass http://disk-image-service;

  # Keep range semantics intact.
  gzip off;
  proxy_buffering off;

  # Timeouts appropriate for large downloads / slow networks.
  proxy_read_timeout 3600s;
  proxy_send_timeout 3600s;
  send_timeout 3600s;

  # CORS preflight must reach the service or be handled consistently here.
  # (If handled here, make sure headers match the backend.)
}
```

If serving images directly from NGINX (no upstream), NGINX supports `Range` for static files by default; still ensure `gzip` is disabled for these locations.

#### CDN notes (CloudFront / Cloudflare / etc.)

If a CDN is in front:

- Ensure it forwards `Range` and does not collapse multiple `Range` requests into a cached 200.
- Ensure CORS response headers are preserved.
- Avoid “automatic compression” features on binary routes.
- See [17 - HTTP Range + CDN Behavior](../areas/storage.md) for CloudFront/Cloudflare limits and an operator validation checklist.

### Security recommendations

Disk images are large, valuable, and easy to abuse. Treat this service as an internet-facing file server unless proven otherwise.

#### Authentication and authorization

Recommended approaches (pick one):

- **Signed URLs** (recommended for public distribution): time-bound, scoped to a single `image_id`.
- **Bearer token/JWT**: validate and authorize per image.
- **mTLS**: for internal deployments (cluster-to-cluster).

Make authorization decisions on a stable identifier (`image_id`), not a filesystem path.

#### Least privilege

- Run the service as a non-root user.
- Grant read-only access to the image store.
- If using S3/GCS/etc., use credentials that can *only* read the required bucket/prefix.
- Keep `/metrics` and admin endpoints behind auth or internal networking.

#### Rate limiting and DoS hardening

- Enforce a maximum concurrent requests per IP/token.
- Enforce a maximum bytes/sec per IP/token if serving publicly.
- Enforce a maximum `Range` length (e.g. 1–8 MiB) to align with the client’s chunking strategy.

#### Image path hardening

If images are on disk:

- Do not accept arbitrary paths from the request.
- Store an allowlist mapping `{image_id -> absolute_path}` in config.
- Reject `..`, `%2e%2e`, and other traversal patterns if paths are ever user-influenced.
- Avoid following symlinks out of the image root.

### LocalFS catalog (`manifest.json`)

For `LocalFS` mode, the image catalog can be driven by a `manifest.json` file stored alongside the
image files. This provides deterministic IDs and friendly names.

Example:

```json
{
  "images": [
    {
      "id": "win7",
      "file": "win7.img",
      "name": "Windows 7 SP1",
      "description": "Clean install",
      "public": true,
      "etag": "\"win7-sp1-x64-v1\"",
      "last_modified": "2026-01-10T00:00:00Z",
      "recommended_chunk_size_bytes": 1048576,
      "content_type": "application/octet-stream"
    }
  ]
}
```

`etag` and `last_modified` are optional. When provided, they override the server’s default
filesystem-derived validators, allowing stable caching for immutable/versioned images even if file
mtimes change during copy/restore.

Notes:

- `etag` must be a valid HTTP **entity-tag**, including quotes (e.g. `"v1"` or `W/"v1"`). Prefer a
  **strong** ETag (no `W/`) so clients can use `If-Range` for safe range resumption.
- `last_modified` must be an RFC3339 timestamp (e.g. `2026-01-10T00:00:00Z`) and must be at or
  after `1970-01-01T00:00:00Z` (pre-epoch times cannot be represented in an HTTP `Last-Modified`
  header).

If no manifest is present, the server may fall back to a stable directory listing (development
only).

### Troubleshooting

#### “Why am I getting 200 OK instead of 206 Partial Content?”

Symptoms:

- Network tab shows `200` for requests that include a `Range` header.
- The browser downloads the entire disk image.
- `StreamingDisk` behaves as if caching is ineffective.

Most common causes:

1. The origin server does not implement `Range` (returns 200 and ignores it).
2. A reverse proxy/CDN strips the `Range` header.
3. Compression or other transformations are enabled (`Content-Encoding`).

How to debug:

```bash
# Does the service return 206 for a trivial range?
curl -v -H 'Range: bytes=0-0' https://disks.examplecdn.com/disks/disk_123/bytes

# Check key headers (you want 206 + Content-Range).
curl -I -H 'Range: bytes=0-0' https://disks.examplecdn.com/disks/disk_123/bytes
```

Expected (example):

```http
HTTP/2 206
accept-ranges: bytes
content-range: bytes 0-0/34359738368
content-length: 1
```

#### CORS preflight failures for `Range`

Symptoms:

- The GET never happens; you only see an `OPTIONS` request.
- Browser console: CORS errors mentioning `Range` or “Request header field range is not allowed…”.

How to debug preflight:

```bash
curl -i -X OPTIONS \
  -H 'Origin: https://app.example.com' \
  -H 'Access-Control-Request-Method: GET' \
  -H 'Access-Control-Request-Headers: range' \
  https://disks.examplecdn.com/disks/disk_123/bytes
```

The response must include:

- `Access-Control-Allow-Origin` matching the requesting origin
- `Access-Control-Allow-Methods` including `GET`
- `Access-Control-Allow-Headers` including `Range` (case-insensitive)

#### “Blocked by Cross-Origin-Embedder-Policy” (COEP) errors

Symptoms:

- Browser console errors like:
  - `Blocked by Cross-Origin-Embedder-Policy: ...`
  - `Cross-Origin-Embedder-Policy policy would block the resource ...`
- `SharedArrayBuffer` becomes unavailable or the page is not cross-origin isolated.

Common causes:

1. App origin has `COOP: same-origin` + `COEP: require-corp`, but the image response is missing CORS headers.
2. The image response has restrictive `Cross-Origin-Resource-Policy` (e.g. `same-origin`) that does not allow the app’s site.
3. Mixed content: app is HTTPS but image is HTTP.

How to debug:

- Confirm the app document response includes COOP/COEP.
- Confirm the image response includes **either**:
  - valid CORS headers **or**
  - a CORP header compatible with the app’s site (`same-site`/`cross-origin`).

## CloudFront: authenticated Range streaming for disk images

This guide describes a concrete AWS setup for serving **large, private disk images** (S3) through **CloudFront** with **authenticated `Range` requests** that work in browsers with **COOP/COEP** (for `SharedArrayBuffer`) and do not leak private data through caching.

Related docs:
- [Disk Image Streaming (HTTP Range + Auth + COOP/COEP)](../areas/storage.md) (normative protocol + CORS/COEP requirements)
- [Disk Image Lifecycle and Access Control](../areas/storage.md) (uploads/ownership/sharing/writeback scopes for hosted service)
- [Remote Disk Image Delivery](../areas/storage.md) (object store + CDN delivery contract)
- [Storage trait consolidation](../areas/storage.md) (repo-wide canonical disk/backend traits)

The intended use case is:

- Disk images stored in **S3** (private bucket).
- Browser reads the image via many **`GET` + `Range: bytes=…`** requests (random access).
- Access control enforced at the **CloudFront edge** using **signed URLs** or **signed cookies**.

---

### A) Architecture

#### High-level diagram

```
Browser (app) ── Range GET/HEAD/OPTIONS ──▶ CloudFront (cdn.example.com)
     ▲                                              │
     │                                              │ (OAC-signed origin request)
     │ Set-Cookie (signed cookies)                  ▼
Backend (auth) ───────────────────────────────▶ S3 bucket (private)
```

The “Backend (auth)” box can be implemented by the reference service at
``services/image-gateway/`` (S3 multipart upload + CloudFront signed cookies/URLs).

#### S3 layout: public vs private keys

Use S3 object keys that make “public vs private” explicit and keep private objects isolated by key prefix:

- **Public** (no auth at CloudFront):
  - `public/base-images/win7-sp1-amd64.raw`
- **Per-user** (CloudFront signed URL/cookie required):
  - `users/<uid>/disk.raw`
  - `users/<uid>/snapshots/<snapshot-id>.raw`

This path split enables two CloudFront cache behaviors:

- `/public/*` → public behavior, long cache TTLs
- `/users/*` → private behavior, viewer access restricted

#### CloudFront cacheable size limits (Range helps; chunking is still a good default)

CloudFront enforces a maximum **cacheable file size per HTTP response**. If a client ever triggers a full-object `GET` (no `Range`) for a very large disk, you can run into CloudFront size-limit behavior (cache bypass or errors depending on settings and size).

`Range` requests help because a `206 Partial Content` response’s `Content-Length` is only the requested slice; CloudFront can cache those slices and serve subsequent requests for the same ranges from the edge cache.

Practical guidance:

- Ensure the browser always reads disk bytes via `Range`.
- For very large objects, prefer learning total size via `Range: bytes=0-0` + `Content-Range` instead of relying on `HEAD` (see [`../areas/storage.md`](../areas/storage.md)).
- For the most predictable behavior across CDNs (and to avoid “first Range miss downloads a lot” surprises), consider storing the disk as **multiple fixed-size chunk objects** instead of one giant object:

```
users/<uid>/images/<image-id>/manifest.json
users/<uid>/images/<image-id>/chunks/000000.bin
users/<uid>/images/<image-id>/chunks/000001.bin
...
```

Then the client maps `byteOffset → chunkIndex + offsetWithinChunk`.

Notes:

- This still works with signed cookies/URLs (scope the policy to `users/<uid>/*`).
- If you fetch whole chunk objects (no `Range` header), you can avoid `Range`-triggered CORS preflights for cross-origin deployments.
- See [`../areas/storage.md`](../areas/storage.md) for more detailed CloudFront Range behavior and chunking guidance.

#### Keep S3 private (OAC/OAI)

S3 must **not** be public. Configure CloudFront to be the only reader:

- Prefer **Origin Access Control (OAC)** (newer).
- Origin Access Identity (OAI) is legacy; use only if you must.

With OAC:

1. Create a CloudFront distribution with an S3 origin.
2. Create/attach an **OAC** to that origin.
3. Add an S3 bucket policy that allows **only** that distribution to `s3:GetObject`.

Example bucket policy (replace placeholders):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "AllowCloudFrontRead",
      "Effect": "Allow",
      "Principal": { "Service": "cloudfront.amazonaws.com" },
      "Action": "s3:GetObject",
      "Resource": "arn:aws:s3:::MY_BUCKET_NAME/*",
      "Condition": {
        "StringEquals": {
          "AWS:SourceArn": "arn:aws:cloudfront::MY_AWS_ACCOUNT_ID:distribution/MY_DISTRIBUTION_ID"
        }
      }
    }
  ]
}
```

#### CloudFront distribution setup (concrete)

##### Use a custom domain (strongly recommended)

Signed cookies are easiest when the CDN hostname is under your domain so cookies are first-party:

- App: `https://app.example.com`
- Disk CDN: `https://cdn.example.com`

Using the default `*.cloudfront.net` hostname makes signed cookies difficult (you generally cannot set cookies for `cloudfront.net` from your own site).

##### Create the distribution and S3 origin

In the distribution:

- **Origin**: your S3 bucket (no public access)
- **Origin access**: attach the OAC created above
- **Viewer protocol policy**: redirect HTTP → HTTPS

##### Add two cache behaviors

Create two behaviors that map directly to the S3 prefixes described above:

- Behavior: `/public/*`
  - Viewer access restriction: **off**
  - Cache TTLs: long (public base images should be immutable; version keys if you update them)
- Behavior: `/users/*`
  - Viewer access restriction: **on** (trusted key group)
  - Cache TTLs: can still be long if per-user images are immutable/versioned

For both behaviors, allow:

- **Allowed HTTP methods**: `GET, HEAD, OPTIONS`
  - `OPTIONS` is required if you will do cross-origin `Range` fetches (preflight).

##### Create signing keys + a key group (for signed URLs/cookies)

CloudFront signed URLs/cookies use an RSA key pair:

1. Generate a key pair:

   ```bash
   openssl genrsa -out cloudfront_private_key.pem 2048
   openssl rsa -pubout -in cloudfront_private_key.pem -out cloudfront_public_key.pem
   ```

2. In CloudFront:
   - Create a **Public key** using `cloudfront_public_key.pem`
   - Create a **Key group** and add that public key
3. In the `/users/*` cache behavior:
   - Set **Trusted key groups** to the key group you created

Your backend stores `cloudfront_private_key.pem` securely and uses the public key ID (often called the “key pair ID” in signing libraries).

##### Forward `Range` to S3 (origin request policy)

Create an **origin request policy** for disk objects that forwards:

- `Range`
- `If-Range` (optional)

If you use an **S3 origin** and expect S3 to answer browser CORS preflights, also forward the CORS preflight headers so S3 can evaluate its CORS rules:

- `Origin`
- `Access-Control-Request-Method`
- `Access-Control-Request-Headers`

Do not forward cookies/query strings to S3 unless you have a specific need.

##### Response headers policy (CORS + CORP)

Attach a response headers policy to disk behaviors that sets:

- CORS headers (if cross-origin)
- `Cross-Origin-Resource-Policy` (CORP) for COEP compatibility

See [Headers policy](#d-headers-policy-cors--corp--coep-compatibility) below.

---

### B) Access control options (signed URLs vs signed cookies)

CloudFront “viewer access restriction” is the key feature here: CloudFront validates the signature **at the edge** before serving *any* bytes (cache hit or origin fetch).

#### Option 1: CloudFront signed URLs

**When to use**

- Cross-origin fetches where cookies are undesirable/unavailable.
- One-off downloads.
- Environments where you cannot or do not want to set cookies (some embedded contexts).

**How it works**

- Your backend generates a URL like:
  - `https://cdn.example.com/users/123/disk.raw?Expires=...&Signature=...&Key-Pair-Id=...`
- Browser uses the same URL for all range reads; the `Range` header changes per request.

**Risks / operational costs**

- Token in URL can leak via:
  - CDN/server logs
  - copy/paste
  - referrers if the URL ever lands in HTML/navigation (mitigate with `Referrer-Policy: no-referrer`)
- Cache fragmentation if you include query strings in the cache key (see caching section).

#### Option 2: CloudFront signed cookies

**When to use (recommended for “browser fetches many ranges”)**

- The emulator will fetch **hundreds/thousands** of ranges.
- You can serve the CDN from a **custom domain on your site** (recommended):
  - `https://cdn.example.com/...` (instead of `https://d123.cloudfront.net/...`)
- You want to avoid secrets in URLs.

**How it works**

- Backend sets 3 cookies (names are fixed by CloudFront):
  - `CloudFront-Policy` (or `CloudFront-Expires` for canned policies)
  - `CloudFront-Signature`
  - `CloudFront-Key-Pair-Id`
- Browser then fetches:
  - `https://cdn.example.com/users/123/disk.raw` with `Range` headers
  - the cookies ride along automatically (same-site), or with `fetch(..., { credentials: "include" })` (cross-origin)

**Pros**

- No token in URL.
- One mint per session instead of per request.
- Better cache hit ratio (cache key does not need to include auth material).

**Cons**

- Requires a custom domain you control if you want cookies to be first-party.
- If the CDN is cross-site, third-party cookie restrictions can break it.

#### Default recommendation

For the browser disk streaming path, default to:

- **Signed cookies**, served from the **same site** as the app (e.g. `app.example.com` + `cdn.example.com`), or ideally the **same origin** (single CloudFront distribution for both app + disks).

Use **signed URLs** when:

- You cannot rely on cookies being sent (cross-site, third-party cookie blocking).
- You need to hand out a link to another client (download tool).

---

### C) Caching & cache keys (and how to avoid private data leakage)

#### Key safety property

With CloudFront signed URLs/cookies, **authorization happens at the edge**, *before* cache lookup is served to the client. That means:

- It is safe for CloudFront to cache `/users/<uid>/disk.raw` and serve it from cache later
  **as long as CloudFront viewer restriction remains enabled** for that path.
- You do **not** need to include auth tokens (cookies/query string) in the cache key to prevent leaks.

#### What to avoid (common footgun)

Do **not** implement disk authorization at the origin using `Authorization: Bearer ...` (or a session cookie) *while leaving CloudFront caching enabled* unless you also:

- include `Authorization` (and/or the cookie) in the cache key **or**
- disable caching for that behavior

Otherwise user A can populate a cached object and user B can receive it without being authorized by the origin.

#### Recommended CloudFront policies (disk behaviors)

**Cache policy**

- **Query strings in cache key:** `None` (recommended)
  - Especially important for signed URLs, where the signature is in the query string.
  - CloudFront still validates the signature; it just won’t create a new cache entry per token.
- **Cookies in cache key:** `None`
  - Signed cookies are validated at edge; including them destroys cache efficiency.
- **Headers in cache key:**
  - Prefer `None`, *except* when required for correctness (see CORS note below).
  - **Do not** include `Range` in the cache key for CloudFront. CloudFront can satisfy later byte-range requests from cache without varying the cache key by `Range`, and varying by `Range` will usually explode cache cardinality for random access.

**Origin request policy**

Forward only what S3 needs for correct partial responses:

- `Range` (required for efficient streaming; otherwise S3 will return the entire object)
- `If-Range` (optional; useful for resumable requests and ETag-based validation)

**CORS note (cache correctness)**

If you set `Access-Control-Allow-Origin` dynamically (echoing the incoming `Origin`), then you must vary by `Origin`:

- include the `Origin` header in the cache key **or**
- don’t echo; instead send a fixed `Access-Control-Allow-Origin: https://app.example.com`

For disk streaming, prefer a fixed allowlist to avoid `Origin` cache fragmentation.

#### Range caching behavior and `CHUNK_SIZE`

Browsers will request many distinct `Range` values.

For CloudFront specifically, keep the cache key minimal (path + version), and rely on CloudFront’s built-in byte-range caching behavior (see also [`../areas/storage.md`](../areas/storage.md)).

On the client side, it is still worth using a fixed `CHUNK_SIZE` to reduce redundant downloads and improve your local caching hit rate (OPFS/sparse cache):

- Choose a fixed `CHUNK_SIZE` in the client (default: **1 MiB**; tune if needed).
- Align all requested ranges to `CHUNK_SIZE` boundaries.

Example alignment logic (matches the approach in `StreamingDisk` in `../areas/storage.md`):

```
chunk_start = floor(byte_offset / CHUNK_SIZE) * CHUNK_SIZE
chunk_end   = ceil(byte_end / CHUNK_SIZE)   * CHUNK_SIZE
Range: bytes={chunk_start}-{chunk_end-1}
```

This makes the client-side “fetch unit” stable, which reduces overlap when different reads land near each other.

---

### D) Headers policy (CORS + CORP + COEP compatibility)

#### Why headers matter here

- `Range` is **not** a CORS-safelisted request header → cross-origin `fetch()` with `Range` triggers a **preflight `OPTIONS`**.
- `Authorization` is also **not** safelisted → if you send it, you trigger preflight as well.
- The main app typically needs **COOP/COEP** to enable `SharedArrayBuffer`. With `Cross-Origin-Embedder-Policy: require-corp`, cross-origin resources must be delivered with **CORS** and/or **CORP** headers that allow them.

#### Main app: COOP/COEP (for `SharedArrayBuffer`)

The emulator page itself (HTML + any workers/scripts that need `SharedArrayBuffer`) is typically served with:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Disk byte responses do not need to set COOP/COEP, but they must be **compatible** with the app’s COEP policy:

- same-origin disk responses are always fine
- cross-origin disk responses must be CORS-enabled and/or send a permissive CORP header (see below)

#### CloudFront Response Headers Policy (disk objects)

Attach a response headers policy to the `/users/*` and `/public/*` cache behaviors with:

##### Byte-stability / anti-transform headers (required for `Range`)

Because disk streaming uses byte offsets, intermediaries **must not** change the wire representation.
Ensure the disk object responses include:

```
Cache-Control: no-transform
Content-Encoding: identity
Content-Type: application/octet-stream
X-Content-Type-Options: nosniff
```

Notes:

- `Cache-Control: no-transform` is defence-in-depth; it tells CDNs/proxies not to apply compression or other transforms.
- Avoid any “automatic compression” features on these cache behaviors. For CloudFront, disable compression for disk behaviors (or ensure it is not applied to `application/octet-stream`).
- `Content-Type` can be set on the S3 object metadata (recommended) or overridden at the edge.
- `Content-Encoding` should be absent or `identity`. Do **not** serve disks as `gzip`/`br`.

##### CORS (for cross-origin disk fetches)

If using signed cookies across origins, you must allow credentials (and you must not use `*` for allow-origin):

```
Access-Control-Allow-Origin: https://app.example.com
Access-Control-Allow-Methods: GET, HEAD, OPTIONS
Access-Control-Allow-Headers: Range, If-Range, If-None-Match, If-Modified-Since, Authorization, Content-Type
Access-Control-Allow-Credentials: true
Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding
Access-Control-Max-Age: 86400
```

If you are using signed URLs and do not need cookies, you can use:

```
Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET, HEAD, OPTIONS
Access-Control-Allow-Headers: Range, If-Range, If-None-Match, If-Modified-Since
Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length, ETag, Content-Encoding
Access-Control-Max-Age: 86400
```

Note: Omit `Access-Control-Allow-Credentials` unless you need credentialed requests; browsers only accept `Access-Control-Allow-Credentials: true`.

##### CORP (for COEP)

Recommended for app+cdn on subdomains of the same registrable domain:

```
Cross-Origin-Resource-Policy: same-site
```

If your app truly is on a different “site” (different eTLD+1), you’ll need:

```
Cross-Origin-Resource-Policy: cross-origin
```

Do **not** use `same-origin` unless the disk responses are same-origin with the app.

#### S3 CORS configuration (when CloudFront forwards preflight)

If CloudFront forwards browser preflights (`OPTIONS`) to S3, the bucket must allow them.

S3 CORS rules are evaluated using:

- `Origin`
- `Access-Control-Request-Method`
- `Access-Control-Request-Headers` (e.g. `range, if-range, authorization`)

A permissive starting point for a single app origin:

```json
[
  {
    "AllowedOrigins": ["https://app.example.com"],
    "AllowedMethods": ["GET", "HEAD", "OPTIONS"],
    "AllowedHeaders": ["Range", "If-Range", "If-None-Match", "If-Modified-Since", "Authorization", "Content-Type", "Origin"],
    "ExposeHeaders": ["Accept-Ranges", "Content-Range", "Content-Length", "ETag", "Content-Encoding"],
    "MaxAgeSeconds": 86400
  }
]
```

Notes:

- If you use **signed cookies** and need credentials, you must use a specific `AllowedOrigins` value (no `*`).
- If you have multiple app origins, either list them explicitly or handle CORS at CloudFront with fixed allow-origins per distribution.

#### Preflight requirements (explicit)

If the browser is cross-origin to the disk URL, a typical preflight looks like:

```
OPTIONS /users/123/disk.raw
Origin: https://app.example.com
Access-Control-Request-Method: GET
Access-Control-Request-Headers: range
```

If you also send `If-Range` (recommended for resumable reads), it becomes:

```
Access-Control-Request-Headers: range, if-range
```

If you send `Authorization`, it becomes:

```
Access-Control-Request-Headers: range, authorization
```

If you also send `If-Range`, include it as well:

```
Access-Control-Request-Headers: range, if-range, authorization
```

Your CloudFront behavior must allow `OPTIONS`, and your response headers must allow `Range` (and `Authorization` if used), otherwise the browser will fail before the first byte is fetched.

---

### E) Example backend code: minting signed cookies / signed URLs

Below is a Node.js example using `@aws-sdk/cloudfront-signer`. The important bits are:

- Use a **custom policy** for signed cookies so you can scope access to a user prefix.
- Set cookies for a domain the browser will send to CloudFront (e.g. `Domain=.example.com` for `cdn.example.com`).

```js
import fs from "node:fs";
import { getSignedCookies, getSignedUrl } from "@aws-sdk/cloudfront-signer";

const keyPairId = process.env.CLOUDFRONT_KEY_PAIR_ID;
const privateKey = fs.readFileSync(process.env.CLOUDFRONT_PRIVATE_KEY_PEM, "utf8");

function epochSeconds(date) {
  return Math.floor(date.getTime() / 1000);
}

// Recommended: signed cookies for many Range requests.
export function mintDiskSignedCookies({ uid, ttlSeconds }) {
  const expires = new Date(Date.now() + ttlSeconds * 1000);
  const resource = `https://cdn.example.com/users/${uid}/*`;

  const policy = JSON.stringify({
    Statement: [
      {
        Resource: resource,
        Condition: {
          DateLessThan: { "AWS:EpochTime": epochSeconds(expires) }
        }
      }
    ]
  });

  // Returns { "CloudFront-Policy": "...", "CloudFront-Signature": "...", "CloudFront-Key-Pair-Id": "..." }
  return getSignedCookies({ keyPairId, privateKey, policy });
}

// Alternative: signed URL (token-in-URL).
export function mintDiskSignedUrl({ uid, ttlSeconds }) {
  const url = `https://cdn.example.com/users/${uid}/disk.raw`;
  const expires = new Date(Date.now() + ttlSeconds * 1000);
  return getSignedUrl({ url, keyPairId, privateKey, dateLessThan: expires });
}
```

Example cookie attributes to use when setting them from your backend:

- `Secure`
- `HttpOnly`
- `Path=/users/<uid>/` (limits where the cookies are sent)
- `Domain=.example.com` (so `app.example.com` can set cookies for `cdn.example.com`)
- `SameSite=Lax` (works for same-site subdomains; for cross-site you may need `SameSite=None; Secure` but this is frequently blocked)

---

### Minimal “do this” checklist

1. S3 bucket is private + blocked public access.
2. CloudFront distribution has S3 origin with **OAC** and bucket policy allows only that distribution.
3. `/users/*` behavior:
   - viewer restriction enabled (trusted key group)
   - allowed methods: `GET, HEAD, OPTIONS`
   - origin request policy forwards `Range` (and optionally `If-Range`)
   - response headers policy sets CORS + `Cross-Origin-Resource-Policy`
4. Backend mints **signed cookies** scoped to `https://cdn.example.com/users/<uid>/*`.
5. Client aligns reads to a fixed `CHUNK_SIZE` to reduce redundant downloads and improve local caching behavior (independent of CloudFront’s cache key).
6. If disks are large enough that CloudFront size limits become relevant, enforce **range-only** access (for non-`HEAD` size discovery, use `Range: bytes=0-0`) or publish them as multiple chunk objects (see [CloudFront cacheable size limits](#cloudfront-cacheable-size-limits-range-helps-chunking-is-still-a-good-default)).

---
