#!/usr/bin/env node
// Download the prebuilt PDFium shared library for the current (or given)
// platform into src-tauri/resources/pdfium/, where Tauri bundles it as a
// resource and the app loads it at runtime for PDF text extraction.
//
//   node scripts/fetch-pdfium.mjs                 # host platform
//   node scripts/fetch-pdfium.mjs mac-univ        # explicit target
//
// Source: https://github.com/bblanchon/pdfium-binaries (BSD-3 / Apache-2.0
// builds of Chromium's PDFium). The version is pinned to match the
// pdfium-render binding feature (`pdfium_latest` → chromium/8125).
import { execFileSync } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, readdirSync, copyFileSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const VERSION = "chromium/8125";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "resources", "pdfium");

function hostTarget() {
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  if (process.platform === "darwin") return `mac-${arch}`;
  if (process.platform === "win32") return `win-${arch}`;
  return `linux-${arch}`;
}

const target = process.argv[2] || process.env.PDFIUM_TARGET || hostTarget();
const libName = target.startsWith("mac") ? "libpdfium.dylib" : target.startsWith("win") ? "pdfium.dll" : "libpdfium.so";
const stamp = join(outDir, ".version");

if (existsSync(join(outDir, libName)) && existsSync(stamp) && readFileSync(stamp, "utf8").trim() === `${VERSION} ${target}`) {
  console.log(`pdfium ${VERSION} (${target}) already present`);
  process.exit(0);
}

const url = `https://github.com/bblanchon/pdfium-binaries/releases/download/${encodeURIComponent(VERSION)}/pdfium-${target}.tgz`;
console.log(`fetching ${url}`);
const res = await fetch(url, { redirect: "follow" });
if (!res.ok) {
  console.error(`download failed: HTTP ${res.status}`);
  process.exit(1);
}
const work = join(tmpdir(), `pdfium-${process.pid}`);
mkdirSync(work, { recursive: true });
const tgz = join(work, "pdfium.tgz");
await pipeline(Readable.fromWeb(res.body), createWriteStream(tgz));
execFileSync("tar", ["-xzf", tgz, "-C", work]);

const libDir = target.startsWith("win") ? join(work, "bin") : join(work, "lib");
const lib = readdirSync(libDir).find((f) => f === libName);
if (!lib) {
  console.error(`${libName} not found in archive (${readdirSync(libDir).join(", ")})`);
  process.exit(1);
}
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
copyFileSync(join(libDir, lib), join(outDir, libName));
for (const lic of ["LICENSE"]) {
  if (existsSync(join(work, lic))) copyFileSync(join(work, lic), join(outDir, "PDFIUM-LICENSE.txt"));
}
writeFileSync(stamp, `${VERSION} ${target}\n`);
rmSync(work, { recursive: true, force: true });
console.log(`pdfium → ${join(outDir, libName)}`);
