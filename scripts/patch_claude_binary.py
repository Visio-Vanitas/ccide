#!/usr/bin/env python3
"""
patch_claude_binary.py — Cross-OS Structural Binary Patcher CLI for Claude Code (`macOS` / `Linux` / `Windows`).

Delegates Bun Mach-O/ELF/PE layout parsing to `scripts/bun_pe.py` and version-independent
regex patch anchors to `scripts/cc_patch_rules.py`.
"""
import argparse
import json
import os
import platform
import shutil
import subprocess
import sys

_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if _SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _SCRIPT_DIR)

from bun_pe import (  # type: ignore[import-not-found]  # noqa: E402
    BunLayout,
    JsModuleSpan,
    clear_bun_bytecode_for_offsets,
    detect_js_syntax_cmd,
    discover_claude_bin,
    parse_bun_layout,
    verify_modified_js_syntax,
)
from cc_patch_rules import (  # type: ignore[import-not-found]  # noqa: E402
    apply_all_structural_patches,
    discover_runtime_symbols,
)

_detect_js_syntax_cmd = detect_js_syntax_cmd
__all__ = [
    "BunLayout",
    "JsModuleSpan",
    "_detect_js_syntax_cmd",
    "parse_bun_layout",
    "patch_buffer_in_memory",
]


def patch_buffer_in_memory(buf: bytearray) -> dict:
    layout = parse_bun_layout(buf)
    orig_snapshots: dict[int, bytes] = {}
    if layout is not None:
        for mod in layout.js_modules:
            orig_snapshots[mod.mod_idx] = bytes(buf[mod.src_start : mod.src_end])
    else:
        orig_snapshots[-1] = bytes(buf)

    syms = discover_runtime_symbols(buf, layout)
    results, patched_offsets = apply_all_structural_patches(buf, layout, syms)
    cleared_modules = clear_bun_bytecode_for_offsets(buf, layout, patched_offsets)
    syntax_errors = verify_modified_js_syntax(
        buf, layout, orig_snapshots, cleared_modules, patched_offsets
    )
    required_ok = all(r["applied"] for r in results if r["required"]) and not syntax_errors

    return {
        "bun_standalone": layout is not None,
        "bun_modules": layout.n_mods if layout else 0,
        "js_modules": len(layout.js_modules) if layout else 0,
        "symbols": syms,
        "required_ok": required_ok,
        "syntax_errors": syntax_errors,
        "patches": results,
        "cleared_bytecode_modules": [{"index": idx, "name": nm} for idx, nm in cleared_modules],
    }


def apply_patches(src_path: str, dst_path: str) -> dict:
    with open(src_path, "rb") as f:
        buf = bytearray(f.read())

    report = patch_buffer_in_memory(buf)
    for p in report["patches"]:
        print(f"  {p['message']}")
    for cm in report["cleared_bytecode_modules"]:
        print(f"  [OK] Cleared Bun bytecode cache for module #{cm['index']}: {cm['name']}")

    if not report["required_ok"]:
        if report.get("syntax_errors"):
            raise RuntimeError("JS syntax validation failed:\n" + "\n".join(report["syntax_errors"]))
        missing = [p["name"] for p in report["patches"] if p["required"] and not p["applied"]]
        raise RuntimeError(f"Required patches did not match: {', '.join(missing)}")

    tmp_dst = dst_path + ".tmp"
    with open(tmp_dst, "wb") as f:
        f.write(buf)
    if platform.system() != "Windows":
        os.chmod(tmp_dst, 0o755)
    if platform.system() == "Darwin" and shutil.which("codesign"):
        subprocess.run(["codesign", "-f", "-s", "-", tmp_dst], check=True)
    try:
        os.replace(tmp_dst, dst_path)
    except PermissionError as exc:
        if os.path.exists(tmp_dst):
            os.remove(tmp_dst)
        raise RuntimeError(
            f"Cannot overwrite {dst_path} (file locked by a running Claude Code process). "
            "Close open Claude windows and re-run `install.ps1 -PatchOnly` or `ccide`."
        ) from exc
    print(f"  [OK] Installed -> {dst_path}")
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description="Cross-OS structural patcher for Claude Code")
    parser.add_argument("--test", action="store_true", help="Write patched binary to temp test path")
    parser.add_argument("--verify", action="store_true", help="Dry-run in memory and output JSON")
    parser.add_argument("--binary", type=str, default="", help="Explicit path to claude binary")
    args = parser.parse_args()

    target_bin = args.binary if args.binary else discover_claude_bin()
    if not os.path.exists(target_bin):
        if args.verify:
            print(json.dumps({"error": f"Binary not found: {target_bin}", "required_ok": False}))
            sys.exit(1)
        print(f"  [SKIP] claude binary not found at {target_bin}; skipping binary patch.")
        sys.exit(0)

    if args.verify:
        bak_cand = target_bin + ".bak"
        src = bak_cand if os.path.exists(bak_cand) and not args.binary else target_bin
        with open(src, "rb") as f:
            buf = bytearray(f.read())
        report = patch_buffer_in_memory(buf)
        report["source_binary"] = src
        print(json.dumps(report, indent=2))
        sys.exit(0 if report["required_ok"] else 1)

    bak_path = target_bin + ".bak"
    with open(target_bin, "rb") as f:
        cur_bytes = f.read()
    is_already_patched = b"g.__cc_h" in cur_bytes or b"/tmp/.cc-ide" in cur_bytes
    if not os.path.exists(bak_path) or not is_already_patched:
        if is_already_patched and not os.path.exists(bak_path):
            print(f"  [WARN] {target_bin} is already patched and has no .bak; reinstall claude-code.")
            sys.exit(0)
        shutil.copy2(target_bin, bak_path)
        print(f"  [OK] Created/refreshed pristine backup -> {bak_path}")

    dst = os.path.join(os.path.dirname(bak_path), "claude-test-bin") if args.test else target_bin
    try:
        apply_patches(bak_path, dst)
        res = subprocess.run([dst, "--version"], capture_output=True, text=True, check=True)
        print(f"  [OK] Verified `{dst} --version` -> {res.stdout.strip()}")
    except RuntimeError as e:
        print(f"  [WARN] Binary patch skipped: {e}")
        sys.exit(0)


if __name__ == "__main__":
    main()
