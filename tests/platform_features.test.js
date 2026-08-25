/**
 * Platform capability detection.
 *
 * There were two copies of `platform/features` — one under `src/`, one under `web/src/` — with two
 * test files between them. The `web/` copy was the evolved one (its WebGPU message knows about the
 * WebGL2 fallback, which the other still denied existed), so it is the one that survives, and the
 * assertions that were unique to the other file moved here with it.
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  detectPlatformFeatures,
  explainMissingRequirements,
} from "../apps/web/src/platform/features.ts";
import { requestWebGpuDevice } from "../apps/web/src/platform/webgpu.ts";
import { getOpfsRoot, OpfsUnavailableError } from "../apps/web/src/platform/opfs.ts";

test("detectPlatformFeatures returns a stable boolean report shape", () => {
  const report = detectPlatformFeatures();
  for (const [key, value] of Object.entries(report)) {
    assert.equal(typeof value, "boolean", `${key} must be boolean`);
  }
});

test("explainMissingRequirements is empty when all capabilities are present", () => {
  const allTrue = {
    crossOriginIsolated: true,
    sharedArrayBuffer: true,
    wasmSimd: true,
    wasmThreads: true,
    jit_dynamic_wasm: true,
    webgpu: true,
    webusb: true,
    webgl2: true,
    opfs: true,
    opfsSyncAccessHandle: true,
    audioWorklet: true,
    offscreenCanvas: true,
  };

  assert.deepEqual(explainMissingRequirements(allTrue), []);
});

test("requestWebGpuDevice fails gracefully when WebGPU is unavailable", async () => {
  await assert.rejects(() => requestWebGpuDevice(), /WebGPU is not available/);
});

test("getOpfsRoot fails gracefully when OPFS is unavailable", async () => {
  await assert.rejects(
    () => getOpfsRoot(),
    (err) => err instanceof OpfsUnavailableError,
  );
});

const GLOBALS = globalThis;

function withGlobals(setup, fn) {
  const prev = {
    crossOriginIsolated: GLOBALS.crossOriginIsolated,
    FileSystemFileHandle: GLOBALS.FileSystemFileHandle,
    navigatorStorage: typeof navigator !== "undefined" ? navigator.storage : undefined,
  };
  try {
    setup();
    return fn();
  } finally {
    if (prev.crossOriginIsolated === undefined) delete GLOBALS.crossOriginIsolated;
    else GLOBALS.crossOriginIsolated = prev.crossOriginIsolated;

    if (prev.FileSystemFileHandle === undefined) delete GLOBALS.FileSystemFileHandle;
    else GLOBALS.FileSystemFileHandle = prev.FileSystemFileHandle;

    if (typeof navigator !== "undefined") {
      if (prev.navigatorStorage === undefined) delete navigator.storage;
      else navigator.storage = prev.navigatorStorage;
    }
  }
}

function report(overrides = {}) {
  return {
    crossOriginIsolated: false,
    sharedArrayBuffer: false,
    wasmSimd: false,
    wasmThreads: false,
    jit_dynamic_wasm: false,
    webgpu: false,
    webusb: false,
    opfs: false,
    opfsSyncAccessHandle: false,
    audioWorklet: false,
    offscreenCanvas: false,
    ...overrides,
  };
}

test("detectPlatformFeatures detects OPFS SyncAccessHandle", () => {
  withGlobals(
    () => {
      // Ensure `opfs: true`.
      navigator.storage = { getDirectory: () => Promise.resolve(null) };

      // `createSyncAccessHandle()` is worker-only at runtime, but for detection it is sufficient
      // to check that the method exists on the prototype.
      GLOBALS.FileSystemFileHandle = class FileSystemFileHandle {
        createSyncAccessHandle() {}
      };
    },
    () => {
      const detected = detectPlatformFeatures();
      assert.equal(detected.opfs, true);
      assert.equal(detected.opfsSyncAccessHandle, true);
    },
  );
});

test("missing SyncAccessHandle message mentions IndexedDB", () => {
  const messages = explainMissingRequirements(
    report({
      crossOriginIsolated: true,
      sharedArrayBuffer: true,
      wasmSimd: true,
      wasmThreads: true,
      jit_dynamic_wasm: true,
      webgpu: true,
      webusb: true,
      opfs: true,
      opfsSyncAccessHandle: false,
      audioWorklet: true,
      offscreenCanvas: true,
    }),
  );

  const text = messages.join("\n");
  assert.ok(/SyncAccessHandle/i.test(text), "expected SyncAccessHandle message");
  assert.ok(/IndexedDB/i.test(text), "expected IndexedDB mention in SyncAccessHandle message");
});
