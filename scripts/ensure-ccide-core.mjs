#!/usr/bin/env node
// Cross-Platform (`macOS` / `Linux` / `Windows pwsh & cmd.exe`) Hook & Binary Resolver for `ccide-core`.
// Node.js (>= 22) is guaranteed present on every Claude Code host, avoiding any `sh`/`zsh` dependency.

import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const isWin = process.platform === "win32";
const binName = isWin ? "ccide-core.exe" : "ccide-core";
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const pluginRoot = process.env.CLAUDE_PLUGIN_ROOT || path.dirname(scriptDir);
const homeDir = process.env.HOME || process.env.USERPROFILE || os.homedir();
const claudeBinDir = path.join(homeDir, ".claude", "bin");
const targetBin = path.join(claudeBinDir, binName);
const repoSlug = process.env.CCIDE_GITHUB_REPO || "Visio-Vanitas/ccide";
const cliArgs = process.argv.slice(2);

function isRunnable(filePath) {
  try {
    const st = fs.statSync(filePath);
    if (!st.isFile() || st.size === 0) return false;
    if (!isWin) fs.accessSync(filePath, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function runEngine(exePath) {
  const child = spawn(exePath, cliArgs, { stdio: "inherit", windowsHide: true });
  child.on("exit", (code) => process.exit(code ?? 0));
  child.on("error", () => process.exit(0));
}

for (const cand of [targetBin, path.join(pluginRoot, "bin", binName)]) {
  if (isRunnable(cand)) {
    runEngine(cand);
    await new Promise(() => {});
  }
}

function resolveTriple() {
  const key = `${process.platform}:${process.arch}`;
  switch (key) {
    case "darwin:arm64":
      return "aarch64-apple-darwin";
    case "darwin:x64":
      return "x86_64-apple-darwin";
    case "linux:x64":
      return "x86_64-unknown-linux-gnu";
    case "linux:arm64":
      return "aarch64-unknown-linux-gnu";
    case "win32:x64":
      return "x86_64-pc-windows-msvc";
    case "win32:arm64":
      return "aarch64-pc-windows-msvc";
    default:
      return null;
  }
}

function fetchBytes(url, redirects = 5) {
  return new Promise((resolve, reject) => {
    if (redirects <= 0) return reject(new Error("too many redirects"));
    const req = https.get(
      url,
      { headers: { "User-Agent": "ccide-bootstrap/0.5" }, timeout: 20000 },
      (res) => {
        if (res.statusCode && res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          res.resume();
          return resolve(fetchBytes(res.headers.location, redirects - 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`HTTP ${res.statusCode}`));
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => resolve(Buffer.concat(chunks)));
      }
    );
    req.on("timeout", () => req.destroy(new Error("timeout")));
    req.on("error", reject);
  });
}

const triple = resolveTriple();
if (!triple) process.exit(0);

const archive = `ccide-core-${triple}.tar.gz`;
const baseUrl = `https://github.com/${repoSlug}/releases/latest/download`;
const lockDir = path.join(claudeBinDir, ".ensure-ccide-core.lock");
let lockHeld = false;
let tmpDir = "";
let stagedBin = "";

async function acquireLock() {
  fs.mkdirSync(claudeBinDir, { recursive: true });
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    try {
      fs.mkdirSync(lockDir);
      lockHeld = true;
      return true;
    } catch (err) {
      if (err && err.code !== "EEXIST") return false;
      try {
        const st = fs.statSync(lockDir);
        if (Date.now() - st.mtimeMs > 60000) {
          fs.rmSync(lockDir, { recursive: true, force: true });
          continue;
        }
      } catch {}
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  return false;
}

try {
  await acquireLock();
  if (isRunnable(targetBin)) {
    if (lockHeld) {
      try { fs.rmSync(lockDir, { recursive: true, force: true }); } catch {}
      lockHeld = false;
    }
    runEngine(targetBin);
    await new Promise(() => {});
  }

  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ccide-dl-"));
  const [tarBuf, sumsBuf] = await Promise.all([
    fetchBytes(`${baseUrl}/${archive}`),
    fetchBytes(`${baseUrl}/SHA256SUMS.txt`),
  ]);

  const sumsTxt = sumsBuf.toString("utf8");
  const match = sumsTxt
    .split(/\r?\n/)
    .map((l) => l.trim().split(/\s+/))
    .find((parts) => parts.length >= 2 && parts[1].replace(/^\*/, "") === archive);
  if (!match) process.exit(0);

  const actual = createHash("sha256").update(tarBuf).digest("hex").toLowerCase();
  if (actual !== match[0].toLowerCase()) process.exit(0);

  const archivePath = path.join(tmpDir, archive);
  fs.writeFileSync(archivePath, tarBuf);
  const tarCmd = isWin ? "tar.exe" : "tar";
  const ext = spawnSync(tarCmd, ["-xzf", archivePath, "-C", tmpDir], { stdio: "ignore", windowsHide: true });
  if (ext.status !== 0) process.exit(0);

  const extracted = path.join(tmpDir, binName);
  if (!fs.existsSync(extracted)) process.exit(0);

  fs.mkdirSync(claudeBinDir, { recursive: true });
  stagedBin = `${targetBin}.tmp.${process.pid}`;
  fs.copyFileSync(extracted, stagedBin);
  if (!isWin) fs.chmodSync(stagedBin, 0o755);
  if (process.platform === "darwin") {
    spawnSync("codesign", ["-f", "-s", "-", stagedBin], { stdio: "ignore" });
  }
  fs.renameSync(stagedBin, targetBin);
  stagedBin = "";

  try {
    const plugBinDir = path.join(pluginRoot, "bin");
    fs.mkdirSync(plugBinDir, { recursive: true });
    const plugBin = path.join(plugBinDir, binName);
    const plugStaged = `${plugBin}.tmp.${process.pid}`;
    fs.copyFileSync(targetBin, plugStaged);
    if (!isWin) fs.chmodSync(plugStaged, 0o755);
    fs.renameSync(plugStaged, plugBin);
  } catch {}
} catch {
  process.exit(0);
} finally {
  if (stagedBin) {
    try { fs.rmSync(stagedBin, { force: true }); } catch {}
  }
  if (tmpDir) {
    try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch {}
  }
  if (lockHeld) {
    try { fs.rmSync(lockDir, { recursive: true, force: true }); } catch {}
  }
}

if (isRunnable(targetBin)) {
  runEngine(targetBin);
}
