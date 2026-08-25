# Guest-code differential fixtures

These are small pieces of compiled Windows code, kept so the interpreter and the
compiled tier can be checked against *real* guest instruction sequences rather
than against hand-written approximations of them. The tests execute the bytes on
the emulated CPU and compare the result with a host implementation of the same
algorithm.

That comparison is the point: several defects were found this way that no
synthetic test would have caught, including a missing add-with-carry in the
compiled tier that produced a wrong-but-plausible 4096-bit signature check, and
a too-generic instruction-fusion matcher that corrupted guest memory.

## What is here

| File | Origin | Used by |
|---|---|---|
| `bcryptprimitives_sha512_compress.bin` | one compiled function — the SHA-512 compression round — from `bcryptprimitives.dll` | the SHA-512 fuse tests, and `interp/tier0/exec.rs` |
| `cryptsp_sha256_compress.bin` | one compiled function — the SHA-256 compression round — from `cryptsp.dll` | `cryptsp_sha256_compress.rs`, and the IR-cache test in `aero-machine` |

These are **not ours**. They are excerpts of Microsoft binaries, included here
solely as test input for interoperability work, and they are pinned by hash in
`scripts/ci/check-repo-policy.sh` so they cannot silently change.

The round-constant tables that used to sit beside them are gone: they were the
published FIPS 180-4 values, and they are now generated in
`crates/aero-cpu-core/src/sha2_constants.rs`, with a test that re-derives them
from the definition.

## What is deliberately absent

`cryptsp_mapped.bin` — a ~92 KB mapped image of `cryptsp.dll` — is **not
committed**. It is not an excerpt; it is most of a DLL, and redistributing it is
a different thing from keeping two small functions for a differential test.

Tests needing it skip when it is missing. To run them, supply a local copy:

```bash
AERO_CRYPTSP_IMAGE=/path/to/cryptsp_mapped.bin cargo test -p aero-cpu-core
```

or place the file at `crates/aero-cpu-core/tests/data/cryptsp_mapped.bin`, where
it is gitignored. It is a flat image of the DLL as mapped at its preferred base,
so guest-relative addresses in the tests line up.

The tests that skip are the streamed SHA-256 update/final path and the
Authenticode PE hasher — the ones that surfaced the unresolved compiled-tier
digest divergence recorded in `wiki/areas/cpu-and-jit.md`.
