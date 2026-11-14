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
const homeDir = isWin
  ? process.env.USERPROFILE || process.env.HOME || os.homedir()
  : process.env.HOME || process.env.USERPROFILE || os.homedir();
const claudeBinDir = path.join(homeDir, ".claude", "bin");
const targetBin = path.join(claudeBinDir, binName);
const repoSlug = process.env.CCIDE_GITHUB_REPO || "Visio-Vanitas/ccide";
const cliArgs = process.argv.slice(2);
const lockDir = path.join(claudeBinDir, ".ensure-ccide-core.lock");
const ownerPidFile = path.join(lockDir, "owner.pid");

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

function findEngine() {
  for (const cand of [targetBin, path.join(pluginRoot, "bin", binName)]) {
    if (isRunnable(cand)) return cand;
  }
  return null;
}

/** Run the engine and resolve only when it exits; the caller's `finally` still runs. */
function runEngine(exePath) {
  return new Promise((resolve) => {
    const child = spawn(exePath, cliArgs, { stdio: "inherit", windowsHide: true });
    child.on("exit", (code, signal) => resolve(code ?? (signal ? 1 : 0)));
    child.on("error", () => resolve(0));
  });
}

function resolveTriple() {
  switch (`${process.platform}:${process.arch}`) {
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
          let next;
          try {
            next = new URL(res.headers.location, url);
          } catch {
            return reject(new Error("bad redirect"));
          }
          if (next.protocol !== "https:") return reject(new Error("insecure redirect"));
          return resolve(fetchBytes(next.toString(), redirects - 1));
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

function readLockOwner() {
  try {
    return fs.readFileSync(ownerPidFile, "utf8").trim();
  } catch {
    return "";
  }
}

async function acquireLock() {
  fs.mkdirSync(claudeBinDir, { recursive: true });
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    try {
      fs.mkdirSync(lockDir);
      fs.writeFileSync(ownerPidFile, String(process.pid));
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

function releaseLock() {
  // Only the owner removes the lock directory; a stale lock from a dead run is
  // reclaimed by the 60s mtime timeout in `acquireLock`.
  if (readLockOwner() !== String(process.pid)) return;
  try {
    fs.rmSync(lockDir, { recursive: true, force: true });
  } catch {}
}

async function main() {
  const existing = findEngine();
  // Resolve the engine path but let the caller launch it *after* `finally` released
  // the lock, so a long-running Claude session never pins the install lock.
  if (existing) return { run: existing };

  const triple = resolveTriple();
  if (!triple) return { code: 0 };

  if (!(await acquireLock())) return { code: 0 };

  let tmpDir = "";
  let stagedBin = "";
  let plugStaged = "";
  try {
    // A concurrent run may have installed the binary while we waited for the lock.
    if (isRunnable(targetBin)) return { run: targetBin };

    const archive = `ccide-core-${triple}.tar.gz`;
    const baseUrl = `https://github.com/${repoSlug}/releases/latest/download`;
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ccide-dl-"));
    const [tarBuf, sumsBuf] = await Promise.all([
      fetchBytes(`${baseUrl}/${archive}`),
      fetchBytes(`${baseUrl}/SHA256SUMS.txt`),
    ]);

    const match = sumsBuf
      .toString("utf8")
      .split(/\r?\n/)
      .map((l) => l.trim().split(/\s+/))
      .find((parts) => parts.length >= 2 && parts[1].replace(/^\*/, "") === archive);
    if (!match) return { code: 0 };

    const actual = createHash("sha256").update(tarBuf).digest("hex").toLowerCase();
    if (actual !== match[0].toLowerCase()) return { code: 0 };

    const archivePath = path.join(tmpDir, archive);
    fs.writeFileSync(archivePath, tarBuf);
    const tarCmd = isWin ? "tar.exe" : "tar";
    const ext = spawnSync(tarCmd, ["-xzf", archivePath, "-C", tmpDir], {
      stdio: "ignore",
      windowsHide: true,
    });
    if (ext.status !== 0) return { code: 0 };

    const extracted = path.join(tmpDir, binName);
    if (!fs.existsSync(extracted)) return { code: 0 };

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
      plugStaged = `${plugBin}.tmp.${process.pid}`;
      fs.copyFileSync(targetBin, plugStaged);
      if (!isWin) fs.chmodSync(plugStaged, 0o755);
      fs.renameSync(plugStaged, plugBin);
      plugStaged = "";
    } catch {
      // Leave `plugStaged` set so `finally` removes the leftover partial copy.
    }

    return { run: targetBin };
  } catch {
    return { code: 0 };
  } finally {
    if (stagedBin) {
      try { fs.rmSync(stagedBin, { force: true }); } catch {}
    }
    if (plugStaged) {
      try { fs.rmSync(plugStaged, { force: true }); } catch {}
    }
    if (tmpDir) {
      try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch {}
    }
    releaseLock();
  }
}

function exitOnSignal(signal) {
  releaseLock();
  process.exit(signal ? 1 : 0);
}

// Signal handlers clean the lock up even when an install is interrupted mid-flight.
for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(sig, () => exitOnSignal(sig));
}

main().then(
  (result) => {
    if (result && result.run && isRunnable(result.run)) {
      return runEngine(result.run).then((code) => process.exit(code ?? 0));
    }
    process.exit((result && result.code) || 0);
  },
  () => process.exit(0)
);
