#!/usr/bin/env python3
"""
bun_pe.py — Cross-OS (`macOS Mach-O` / `Linux ELF` / `Windows PE`) Bun Standalone Layout & JS Checker.

Handles:
1. Dynamic `\\n---- Bun! ----\\n` trailer discovery across the trailing 64 MB of any standalone
   binary, including Windows PE `.exe` files that append Authenticode certificate tables or
   section alignment padding after the Bun trailer!
2. Cross-OS virtual filesystem prefixes in `StandaloneModuleGraph`:
   - Unix (`macOS` / `Linux`): `/$bunfs/root/...`
   - Windows (`win32-x64` / `win32-arm64`): `B:/~BUN/root/...` and `B:\\~BUN\\root\\...`
3. V8 / Node ESM syntax verification on modified JS modules.
"""
import glob
import os
import platform
import shutil
import struct
import subprocess
from typing import NamedTuple


class JsModuleSpan(NamedTuple):
    mod_idx: int
    name: str
    src_start: int
    src_end: int


class BunLayout(NamedTuple):
    base: int
    table_start: int
    n_mods: int
    js_modules: list[JsModuleSpan]


def is_bun_virtual_js_path(name: str) -> bool:
    norm = name.replace("\\", "/")
    return (
        norm.startswith("/$bunfs/")
        or norm.startswith("B:/~BUN/")
        or norm.startswith("b:/~bun/")
        or "/~BUN/" in norm
    )


def discover_claude_bin() -> str:
    env_bin = os.environ.get("CCIDE_CLAUDE_BIN", "").strip()
    if env_bin and os.path.exists(env_bin):
        return env_bin

    which_claude = shutil.which("claude") or shutil.which("claude.cmd") or shutil.which("claude.exe")
    if which_claude:
        real_claude = os.path.realpath(which_claude)
        base_dir = os.path.dirname(real_claude)
        for cand in (
            os.path.join(base_dir, "bin", "claude.exe"),
            os.path.join(base_dir, "node_modules", "@anthropic-ai", "claude-code", "bin", "claude.exe"),
            real_claude,
        ):
            if os.path.isfile(cand) and not cand.lower().endswith((".cmd", ".ps1", ".bat")):
                if os.path.getsize(cand) > 1024 * 1024:
                    return cand

    if platform.system() == "Windows":
        appdata = os.environ.get("APPDATA", "")
        localappdata = os.environ.get("LOCALAPPDATA", "")
        win_globs = [
            os.path.join(appdata, "npm", "node_modules", "@anthropic-ai", "claude-code*", "bin", "claude.exe"),
            os.path.join(appdata, "npm", "node_modules", "@anthropic-ai", "claude-code-win32-*", "claude.exe"),
            os.path.join(localappdata, "Claude", "**", "claude.exe"),
            os.path.expanduser("~/.local/bin/claude.exe"),
        ]
        for pat in win_globs:
            hits = sorted(glob.glob(pat, recursive=True))
            big = [h for h in hits if os.path.isfile(h) and os.path.getsize(h) > 5 * 1024 * 1024]
            if big:
                return big[-1]
        return os.path.expanduser("~/.local/bin/claude.exe")

    matches = sorted(
        glob.glob(
            os.path.expanduser(
                "~/.nvm/versions/node/*/lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe"
            )
        )
    )
    if matches:
        return matches[-1]
    return os.path.expanduser("~/.local/bin/claude")


def _try_parse_marker_at(buf: bytes | bytearray, mpos: int) -> BunLayout | None:
    if mpos < 32:
        return None
    byte_count, mod_off, mod_len = struct.unpack_from("<QII", buf, mpos - 32)
    base = mpos - 32 - byte_count
    table_start = base + mod_off
    if base < 0 or mod_len == 0 or mod_len % 52 != 0 or table_start + mod_len > len(buf):
        return None

    n_mods = mod_len // 52
    if n_mods > 500_000:
        return None
    js_modules: list[JsModuleSpan] = []
    for i in range(n_mods):
        entry_off = table_start + i * 52
        entry = struct.unpack_from("<13I", buf, entry_off)
        name_s, name_l, code_s, code_l = entry[0], entry[1], entry[2], entry[3]
        if base + name_s + name_l > len(buf) or base + code_s + code_l > len(buf):
            continue
        name = bytes(buf[base + name_s : base + name_s + name_l]).decode("utf-8", errors="ignore")
        if is_bun_virtual_js_path(name) and code_l > 0:
            js_modules.append(JsModuleSpan(i, name, base + code_s, base + code_s + code_l))

    if not js_modules:
        return None
    return BunLayout(base=base, table_start=table_start, n_mods=n_mods, js_modules=js_modules)


def parse_bun_layout(buf: bytes | bytearray) -> BunLayout | None:
    """
    Scans backward for `\\n---- Bun! ----\\n` (tolerating trailing Windows PE Authenticode
    signature blocks or padding) and validates the preceding 32-byte `StandaloneModuleGraph` header.
    """
    marker = b"\n---- Bun! ----\n"
    search_end = len(buf)
    tail_floor = max(0, len(buf) - 64 * 1024 * 1024)

    while search_end > tail_floor:
        mpos = buf.rfind(marker, tail_floor, search_end)
        if mpos == -1:
            break
        parsed = _try_parse_marker_at(buf, mpos)
        if parsed is not None:
            return parsed
        search_end = mpos

    mpos = buf.rfind(marker, 0, tail_floor)
    if mpos != -1:
        return _try_parse_marker_at(buf, mpos)
    return None


def clear_bun_bytecode_for_offsets(
    buf: bytearray, layout: BunLayout | None, patched_offsets: list[int]
) -> list[tuple[int, str]]:
    if layout is None or not patched_offsets:
        return []
    cleared: list[tuple[int, str]] = []
    for mod in layout.js_modules:
        if any(mod.src_start <= off < mod.src_end for off in patched_offsets):
            entry_off = layout.table_start + mod.mod_idx * 52
            entry = list(struct.unpack_from("<13I", buf, entry_off))
            entry[6] = 0
            entry[7] = 0
            entry[12] &= ~0x100
            struct.pack_into("<13I", buf, entry_off, *entry)
            cleared.append((mod.mod_idx, mod.name))
    return cleared


def iter_js_regions(buf: bytearray, layout: BunLayout | None) -> list[tuple[int, memoryview]]:
    mv = memoryview(buf)
    if layout is not None:
        return [(m.src_start, mv[m.src_start : m.src_end]) for m in layout.js_modules]
    return [(0, mv)]


def detect_js_syntax_cmd() -> list[str] | None:
    node_bin = shutil.which("node")
    if not node_bin:
        return None
    probe = subprocess.run(
        [node_bin, "--js-explicit-resource-management", "-e", ""],
        capture_output=True,
    )
    if probe.returncode == 0:
        return [node_bin, "--js-explicit-resource-management", "--input-type=module", "--check"]
    return [node_bin, "--input-type=module", "--check"]


def verify_modified_js_syntax(
    buf: bytearray,
    layout: BunLayout | None,
    orig_snapshots: dict[int, bytes],
    cleared_modules: list[tuple[int, str]],
    patched_offsets: list[int],
) -> list[str]:
    syntax_errors: list[str] = []
    js_check_cmd = detect_js_syntax_cmd()
    if js_check_cmd is None or not patched_offsets:
        return syntax_errors

    if layout is not None and cleared_modules:
        cleared_idx_set = {idx for idx, _ in cleared_modules}
        for mod in layout.js_modules:
            if mod.mod_idx in cleared_idx_set:
                orig_bytes = orig_snapshots.get(mod.mod_idx, b"")
                base_ok = (
                    subprocess.run(js_check_cmd, input=orig_bytes, capture_output=True).returncode
                    == 0
                )
                if base_ok:
                    mod_bytes = bytes(buf[mod.src_start : mod.src_end])
                    proc = subprocess.run(js_check_cmd, input=mod_bytes, capture_output=True)
                    if proc.returncode != 0:
                        err_txt = proc.stderr.decode("utf-8", errors="replace").strip()
                        syntax_errors.append(f"Module #{mod.mod_idx} ({mod.name}): {err_txt}")
    elif layout is None:
        orig_bytes = orig_snapshots.get(-1, b"")
        if subprocess.run(js_check_cmd, input=orig_bytes, capture_output=True).returncode == 0:
            proc = subprocess.run(js_check_cmd, input=bytes(buf), capture_output=True)
            if proc.returncode != 0:
                err_txt = proc.stderr.decode("utf-8", errors="replace").strip()
                syntax_errors.append(f"Bundle: {err_txt}")
    return syntax_errors
