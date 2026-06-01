#!/usr/bin/env node
// Cross-platform clean (re)build for Document Finder — Windows, macOS, Linux.
//
//   pnpm clean-build        # clean app crate + caches, release-fast build
//   pnpm clean-build:full   # also nuke native deps (llama.cpp/ONNX); 15-25 min
//   pnpm clean-build:dev    # debug build + hot reload (tauri dev)
//
// One command on every OS. Node is the entry point (not a pnpm shell script)
// because `rm -rf` in a pnpm script isn't cmd.exe-safe.
//
// Two spawn mechanisms, on purpose:
//   * cargo / pnpm  -> run through a SHELL. On Windows `pnpm` is `pnpm.cmd`, a
//                      batch shim; Node's shell-less spawner only finds real
//                      .exe files and dies with ENOENT, so a shell is required
//                      for PATHEXT resolution.
//   * tauri build   -> run node on the Tauri CLI's own entry DIRECTLY (no pnpm,
//                      no shell). The CLI's `--profile` flag must reach cargo
//                      through a single `--` separator; threading that through
//                      `pnpm exec` is what broke the build (pnpm eats one `--`,
//                      and how many survive is shell-dependent). Calling node
//                      with an explicit argv array makes `--` a literal token.

import { execSync, execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
// Resolve the Tauri CLI's Node entry (the same file `pnpm tauri` runs).
const tauriCli = require.resolve("@tauri-apps/cli/tauri.js");

const args = new Set(process.argv.slice(2));
const full = args.has("--full");
const dev = args.has("--dev");

// Run a command string through the platform shell. Required for `pnpm`
// (pnpm.cmd on Windows) and convenient for `cargo`. Args here are static and
// space-free, so the shell string is safe.
function sh(cmd) {
  console.log(`\n==> ${cmd}`);
  execSync(cmd, { stdio: "inherit", cwd: root });
}

// Invoke the Tauri CLI's Node entry directly — no pnpm, no shell — so the
// single `--` separator reaches cargo intact regardless of platform.
function tauri(tauriArgs) {
  console.log(`\n==> node tauri.js ${tauriArgs.join(" ")}`);
  execFileSync(process.execPath, [tauriCli, ...tauriArgs], {
    stdio: "inherit",
    cwd: root,
    shell: false,
  });
}

function wipe(rel) {
  console.log(`==> remove ${rel}`);
  rmSync(join(root, rel), { recursive: true, force: true });
}

// Stop any running app instance so the clean can delete the locked binary. On
// Windows `cargo clean` fails with "Access is denied (os error 5)" while the
// app holds document-finder.exe. taskkill/pkill exit non-zero when nothing
// matches — that's expected, so swallow it.
function killApp() {
  const cmds =
    process.platform === "win32"
      ? ['taskkill /IM "document-finder.exe" /F', 'taskkill /IM "Document Finder.exe" /F']
      : ['pkill -x "Document Finder"', "pkill -x document-finder"];
  for (const cmd of cmds) {
    try {
      execSync(cmd, { stdio: "ignore" });
    } catch {
      // No matching process — fine.
    }
  }
}

console.log(`==> Document Finder clean rebuild${full ? " (full)" : ""}${dev ? " (dev)" : ""}`);

// 0. Stop the running app first (see killApp) so step 1's clean can remove the
//    locked binary instead of dying on it.
killApp();

// 1. Clean the Rust build. Default keeps the slow llama.cpp/ONNX native builds
//    cached (finishes in minutes); --full nukes everything (15-25 min cold).
sh(
  full
    ? "cargo clean --manifest-path src-tauri/Cargo.toml"
    : "cargo clean -p document-finder --manifest-path src-tauri/Cargo.toml",
);

// 2. Clear frontend build caches so Vite/tsc rebuild fresh.
wipe("dist");
wipe("node_modules/.vite");

// 3. Reinstall JS deps (esbuild's binary ships in its platform package, so
//    --ignore-scripts is fine; see allowBuilds in pnpm-workspace.yaml).
sh("pnpm install --frozen-lockfile --ignore-scripts");

// 4. Build (or dev). `build -- --profile release-fast`: the `--` is a literal
//    array element, so the Tauri CLI forwards `--profile release-fast` to cargo.
//    (tauri.conf.json's beforeBuildCommand runs `pnpm build` = vite.)
if (dev) {
  tauri(["dev"]);
} else {
  tauri(["build", "--", "--profile", "release-fast"]);
}

console.log("\n==> Done.");
