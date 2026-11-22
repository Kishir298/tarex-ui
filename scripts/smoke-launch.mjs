// Advisory launch smoke test used by the `smoke` CI job on Windows and Linux.
//
// Builds produce `src-tauri/target/debug/tarex-ui` (`.exe` on Windows) via
// `tauri build --debug --no-bundle`. This script starts it, waits a grace
// period, and fails if the process died early or printed a panic. It does
// not assert UI state — a GUI launch can fail for runner reasons (display,
// GPU, webkit build) that have nothing to do with the code, which is why the
// CI job that runs this is advisory.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = fileURLToPath(new URL("..", import.meta.url));
const binaryName = process.platform === "win32" ? "tarex-ui.exe" : "tarex-ui";
const binary = join(repoRoot, "src-tauri", "target", "debug", binaryName);

if (!existsSync(binary)) {
  console.error(`smoke: binary not found at ${binary}`);
  process.exit(1);
}

const graceMs = Number(process.env.SMOKE_GRACE_MS ?? 30000);
// Positive assertion: a debug build logs INFO to stdout, and `pty opened`
// proves the PTY subsystem really came up rather than the process merely
// staying alive. Override with SMOKE_MARKER="" to fall back to "no panic".
const marker = process.env.SMOKE_MARKER ?? "pty opened";

const child = spawn(binary, [], {
  stdio: ["ignore", "pipe", "pipe"],
});

let output = "";
child.stdout?.on("data", (chunk) => {
  output += chunk;
  process.stdout.write(chunk);
});
child.stderr?.on("data", (chunk) => {
  output += chunk;
  process.stderr.write(chunk);
});

let exitInfo = null;
child.on("exit", (code, signal) => {
  exitInfo = { code, signal };
});

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function fail(reason) {
  console.error(`smoke: ${reason}`);
  console.error("---- last 4000 chars of output ----");
  console.error(output.slice(-4000));
  process.exit(1);
}

async function main() {
  await sleep(graceMs);

  if (exitInfo) {
    fail(`process exited early (code=${exitInfo.code} signal=${exitInfo.signal})`);
  }
  if (/panicked at/i.test(output)) {
    fail("panic detected in output");
  }
  if (marker && !new RegExp(marker).test(output)) {
    fail(`expected marker not found: ${marker}`);
  }

  console.log(`smoke: process still alive after ${graceMs}ms with no panic`);
  child.kill("SIGTERM");
  await sleep(3000);
  if (!exitInfo) {
    console.log("smoke: forcing shutdown after SIGTERM grace");
    child.kill("SIGKILL");
    await sleep(1000);
  }
  process.exit(0);
}

main().catch((error) => fail(String(error)));
