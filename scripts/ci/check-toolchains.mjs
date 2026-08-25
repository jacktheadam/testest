import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

function fallbackFormatOneLineError(err, maxLen = 512) {
    let msg = "Error";
    try {
        if (typeof err === "string") msg = err;
        else if (err && err.message && typeof err.message === "string") msg = err.message;
    } catch {
        // ignore hostile getters
    }
    try {
        msg = String(msg).replace(/\s+/gu, " ").trim();
    } catch {
        msg = "Error";
    }
    if (!Number.isInteger(maxLen) || maxLen <= 0) return "";
    if (msg.length > maxLen) msg = msg.slice(0, maxLen);
    return msg || "Error";
}

let formatOneLineError = fallbackFormatOneLineError;
try {
    const mod = await import(new URL("../../packages/transport-safety/src/text.js", import.meta.url));
    if (typeof mod?.formatOneLineError === "function") {
        formatOneLineError = mod.formatOneLineError;
    }
} catch {
    // ignore - fallback stays active
}

function fail(message) {
    console.error(`toolchain check failed: ${message}`);
    process.exit(1);
}

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "../..");

const rustToolchainTomlPath = path.join(repoRoot, "rust-toolchain.toml");
const toolchainsJsonPath = path.join(repoRoot, "scripts/toolchains.json");
const wasmBuildScriptPath = path.join(repoRoot, "apps/web/scripts/build_wasm.mjs");
const justfilePath = path.join(repoRoot, "justfile");

const rustToolchainToml = readFileSync(rustToolchainTomlPath, "utf8");
// Allow trailing comments after the channel assignment so the file can be annotated without
// breaking policy checks.
const channelMatch = rustToolchainToml.match(/^\s*channel\s*=\s*"([^"]+)"\s*(?:#.*)?$/m);
if (!channelMatch) {
    fail(`Unable to find [toolchain].channel in ${rustToolchainTomlPath}`);
}

const stableChannel = channelMatch[1].trim();
if (!/^\d+\.\d+\.\d+$/.test(stableChannel)) {
    fail(
        `rust-toolchain.toml must pin stable to an explicit version (expected 1.xx.y; got '${stableChannel}'). ` +
            "See wiki: rust toolchain policy.",
    );
}

let toolchains;
try {
    toolchains = JSON.parse(readFileSync(toolchainsJsonPath, "utf8"));
} catch (err) {
    fail(`Failed to parse ${toolchainsJsonPath}: ${formatOneLineError(err, 512)}`);
}

const wasmNightly = toolchains?.rust?.nightlyWasm;
if (typeof wasmNightly !== "string" || wasmNightly.trim() === "") {
    fail(`scripts/toolchains.json must define rust.nightlyWasm (string)`);
}
if (!/^nightly-\d{4}-\d{2}-\d{2}$/.test(wasmNightly.trim())) {
    fail(`rust.nightlyWasm must be pinned to a nightly date (nightly-YYYY-MM-DD); got '${wasmNightly}'`);
}

const wasmBuildScript = readFileSync(wasmBuildScriptPath, "utf8");
if (!wasmBuildScript.includes("scripts/toolchains.json") || !wasmBuildScript.includes("nightlyWasm")) {
    fail(
        `web/scripts/build_wasm.mjs must load the pinned nightly toolchain from scripts/toolchains.json (rust.nightlyWasm).`,
    );
}

if (/\+nightly(?!-)/.test(wasmBuildScript)) {
    fail("apps/web/scripts/build_wasm.mjs uses unpinned '+nightly' (must use a pinned nightly-YYYY-MM-DD toolchain).");
}
if (/env\.RUSTUP_TOOLCHAIN\s*=\s*["']/.test(wasmBuildScript)) {
    fail("apps/web/scripts/build_wasm.mjs sets RUSTUP_TOOLCHAIN to a string literal (must come from scripts/toolchains.json).");
}
if (!/env\.RUSTUP_TOOLCHAIN\s*=\s*wasmThreadedToolchain\b/.test(wasmBuildScript)) {
    fail(
        "apps/web/scripts/build_wasm.mjs must set env.RUSTUP_TOOLCHAIN from the pinned toolchain loaded from scripts/toolchains.json " +
            "(expected assignment to wasmThreadedToolchain).",
    );
}

const justfile = readFileSync(justfilePath, "utf8");
if (!justfile.includes("scripts/toolchains.json") || !justfile.includes("nightlyWasm")) {
    fail(`justfile must read the pinned nightly toolchain from scripts/toolchains.json (rust.nightlyWasm).`);
}
const justfileLines = justfile.split(/\r?\n/);
for (let i = 0; i < justfileLines.length; i += 1) {
    const line = justfileLines[i];
    const trimmed = line.trimStart();
    if (trimmed === "" || trimmed.startsWith("#")) {
        continue;
    }

    if (/\brustup\s+toolchain\s+install\s+nightly(?!-)/.test(trimmed)) {
        fail(`justfile:${i + 1} installs unpinned nightly; use scripts/toolchains.json (rust.nightlyWasm).`);
    }
    if (/--toolchain\s+nightly(?!-)/.test(trimmed)) {
        fail(`justfile:${i + 1} references unpinned '--toolchain nightly'; use scripts/toolchains.json (rust.nightlyWasm).`);
    }
}

process.stdout.write(
    [
        "toolchain check ok",
        `- stable: ${stableChannel} (pinned)`,
        `- nightly wasm: ${wasmNightly.trim()} (pinned)`,
        "",
    ].join("\n"),
);
