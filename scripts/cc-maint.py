#!/usr/bin/env python3
"""
cc-maint.py — Semi-Automated Cross-OS Symbol & Schema Maintenance Workbench (`ccide --check-cc`).

Concurrently downloads, SHA1-verifies (`dist.shasum`), and audits `@anthropic-ai/claude-code`
across `darwin-arm64`, `linux-x64`, and `win32-x64` (`B:/~BUN/root/claude.exe` PE) in seconds!
"""
from concurrent.futures import ThreadPoolExecutor
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if _SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _SCRIPT_DIR)

from bun_pe import discover_claude_bin, parse_bun_layout  # type: ignore[import-not-found]  # noqa: E402
from cc_schema_inspector import (  # type: ignore[import-not-found]  # noqa: E402
    collect_js_corpus,
    diagnose_missed_patches,
    extract_live_schema,
)
from patch_claude_binary import patch_buffer_in_memory  # type: ignore[import-not-found]  # noqa: E402

REPO_ROOT = os.path.dirname(_SCRIPT_DIR)
SCHEMA_PATH = os.path.join(REPO_ROOT, "assets", "cc-schema.json")
DEFAULT_PLATFORMS = ("darwin-arm64", "linux-x64", "win32-x64")
NPM_REGISTRIES = ("https://registry.npmjs.org", "https://registry.npmmirror.com")


def load_baseline() -> dict:
    with open(SCHEMA_PATH, encoding="utf-8") as f:
        return json.load(f)


def pick_fastest_registry() -> str:
    def probe(base: str) -> tuple[float, str]:
        t0 = time.monotonic()
        try:
            req = urllib.request.Request(
                f"{base}/@anthropic-ai/claude-code/latest",
                headers={"Accept": "application/json", "User-Agent": "ccide-maint/0.7"},
            )
            with urllib.request.urlopen(req, timeout=4.0) as resp:
                resp.read(256)
            return (time.monotonic() - t0, base)
        except Exception:
            return (999.0, base)

    with ThreadPoolExecutor(max_workers=len(NPM_REGISTRIES)) as ex:
        ranked = sorted(ex.map(probe, NPM_REGISTRIES), key=lambda x: x[0])
    return ranked[0][1] if ranked[0][0] < 900.0 else NPM_REGISTRIES[0]


def fetch_json_url(url: str) -> dict:
    req = urllib.request.Request(
        url, headers={"Accept": "application/json", "User-Agent": "ccide-maint/0.7"}
    )
    with urllib.request.urlopen(req, timeout=15.0) as resp:
        return json.loads(resp.read().decode("utf-8"))


def resolve_npm_version(version_spec: str, reg: str) -> str:
    if version_spec and version_spec != "latest":
        return version_spec
    meta = fetch_json_url(f"{reg}/@anthropic-ai/claude-code/latest")
    return str(meta["version"])


def unpack_npm_platform_binary(plat: str, cc_ver: str, reg: str, dest_dir: str) -> str | None:
    pkg = f"@anthropic-ai/claude-code-{plat}"
    # Always fetch authoritative shasum from official registry first (fallback to `reg` if unreachable)
    try:
        meta = fetch_json_url(f"https://registry.npmjs.org/{pkg}/{cc_ver}")
    except Exception:
        meta = fetch_json_url(f"{reg}/{pkg}/{cc_ver}")
    expected_sha1 = str(meta.get("dist", {}).get("shasum", "")).lower()
    official_tar = str(meta.get("dist", {}).get("tarball", ""))
    mirror_meta = fetch_json_url(f"{reg}/{pkg}/{cc_ver}") if reg != "https://registry.npmjs.org" else meta
    cand_urls = [str(mirror_meta.get("dist", {}).get("tarball", "")), official_tar]

    tgz_path = os.path.join(dest_dir, f"{plat}.tgz")
    curl_bin = shutil.which("curl")
    downloaded = False
    for u in [x for x in cand_urls if x]:
        try:
            if curl_bin:
                subprocess.run(
                    [curl_bin, "-fsSL", "--connect-timeout", "8", "--max-time", "120", "-o", tgz_path, u],
                    check=True,
                )
            else:
                urllib.request.urlretrieve(u, tgz_path)
            if expected_sha1:
                h = hashlib.sha1()
                with open(tgz_path, "rb") as f:
                    for chunk in iter(lambda: f.read(1 << 20), b""):
                        h.update(chunk)
                if h.hexdigest().lower() != expected_sha1:
                    continue
            downloaded = True
            break
        except Exception:
            continue
    if not downloaded:
        return None

    extract_dir = os.path.join(dest_dir, "unpacked")
    os.makedirs(extract_dir, exist_ok=True)
    with tarfile.open(tgz_path, "r:gz") as tf:
        for member in tf.getmembers():
            if member.isfile() and member.size > 5 * 1024 * 1024:
                out_file = os.path.join(extract_dir, os.path.basename(member.name))
                src_f = tf.extractfile(member)
                if src_f is not None:
                    with open(out_file, "wb") as dst_f:
                        shutil.copyfileobj(src_f, dst_f)
                    return out_file
    return None


def inspect_binary_file(bin_path: str, label: str, baseline: dict, include_schema: bool) -> dict:
    with open(bin_path, "rb") as f:
        orig_bytes = f.read()
    orig_buf = bytearray(orig_bytes)
    corpus = collect_js_corpus(orig_buf, parse_bun_layout(orig_buf))
    rep = patch_buffer_in_memory(bytearray(orig_bytes))
    missed = diagnose_missed_patches(rep, corpus, baseline.get("patch_anchors", {}))
    out = {
        "target": label,
        "binary": bin_path,
        "bun_standalone": rep["bun_standalone"],
        "js_modules": rep["js_modules"],
        "symbols": rep["symbols"],
        "required_ok": rep["required_ok"],
        "all_optional_ok": len(missed) == 0,
        "syntax_errors": rep["syntax_errors"],
        "missed_diagnostics": missed,
    }
    if include_schema:
        out["schema_diff"] = extract_live_schema(corpus, baseline)
    return out


def apply_baseline_update(baseline: dict, schema_diff: dict, cc_ver: str) -> None:
    if cc_ver:
        baseline["baseline_cc_version"] = cc_ver
    for sec in ("hook_events", "statusline_keys", "usage_keys", "subagent_meta_keys"):
        cur = set(baseline.get(sec, []))
        cur.update(schema_diff.get(sec, {}).get("added", []))
        baseline[sec] = sorted(cur)
    with open(SCHEMA_PATH, "w", encoding="utf-8") as f:
        json.dump(baseline, f, indent=2)
        f.write("\n")


def print_human_summary(summary: dict) -> None:
    print(f"=== cc-ide Claude Code Maintenance Workbench (v{summary.get('cc_version', 'local')}) ===")
    for t in summary["targets"]:
        st = "OK" if (t["required_ok"] and t["all_optional_ok"]) else ("WARN" if t["required_ok"] else "FAIL")
        print(
            f"  [{st}] {t['target']:<14} modules={t['js_modules']} symbols={t['symbols']} "
            f"syntax_err={len(t['syntax_errors'])}"
        )
        for diag in t["missed_diagnostics"]:
            tag = "REQUIRED" if diag["required"] else "OPTIONAL"
            print(f"        -> [{tag} MISS] {diag['patch']} (anchor={diag['anchor']!r}):\n           {diag['context_snippet'][:200]}")
        for sec, d in t.get("schema_diff", {}).items():
            if d["added"] or d["removed"]:
                print(f"        -> [SCHEMA {sec}] +{d['added']} -{d['removed']}")


def main() -> None:
    ap = argparse.ArgumentParser(description="cc-ide Cross-OS Claude Code Symbol & Schema Workbench")
    ap.add_argument("--local", action="store_true", help="Inspect locally installed Claude Code binary")
    ap.add_argument("--binary", type=str, default="", help="Inspect an explicit binary/bundle path")
    ap.add_argument("--fetch-npm", nargs="?", const="latest", default="", help="Probe npm 3-OS packages")
    ap.add_argument("--platforms", type=str, default=",".join(DEFAULT_PLATFORMS))
    ap.add_argument("--update-baseline", action="store_true")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    baseline = load_baseline()
    targets_out: list[dict] = []
    cc_ver = "local"

    if args.fetch_npm:
        reg = pick_fastest_registry()
        cc_ver = resolve_npm_version(args.fetch_npm, reg)
        plats = [p.strip() for p in args.platforms.split(",") if p.strip()]
        with tempfile.TemporaryDirectory(prefix="ccide-maint-") as tmp_root:
            def probe_one(idx_plat: tuple[int, str]) -> dict | None:
                idx, plat = idx_plat
                pdir = os.path.join(tmp_root, plat)
                os.makedirs(pdir, exist_ok=True)
                bp = unpack_npm_platform_binary(plat, cc_ver, reg, pdir)
                return inspect_binary_file(bp, plat, baseline, idx == 0) if bp else None

            with ThreadPoolExecutor(max_workers=max(1, len(plats))) as ex:
                for res in ex.map(probe_one, enumerate(plats)):
                    if res is not None:
                        targets_out.append(res)
    else:
        cand = args.binary or discover_claude_bin()
        src = (cand + ".bak") if (not args.binary and os.path.isfile(cand + ".bak")) else cand
        if not os.path.isfile(src):
            print(json.dumps({"error": f"Claude binary not found: {src}", "required_ok": False}))
            sys.exit(1)
        targets_out.append(inspect_binary_file(src, "local", baseline, True))

    symbol_sets = [json.dumps(t["symbols"], sort_keys=True) for t in targets_out]
    summary = {
        "schema_version": 1,
        "cc_version": cc_ver,
        "required_ok": all(t["required_ok"] for t in targets_out) and bool(targets_out),
        "all_optional_ok": all(t["all_optional_ok"] for t in targets_out),
        "symbols_identical_across_os": len(set(symbol_sets)) <= 1,
        "targets": targets_out,
    }
    if args.update_baseline and targets_out and "schema_diff" in targets_out[0]:
        apply_baseline_update(baseline, targets_out[0]["schema_diff"], cc_ver if cc_ver != "local" else "")
    print(json.dumps(summary, indent=2) if args.json else "", end="")
    if not args.json:
        print_human_summary(summary)
    sys.exit(0 if summary["required_ok"] else 1)


if __name__ == "__main__":
    main()
