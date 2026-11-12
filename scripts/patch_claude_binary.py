#!/usr/bin/env python3
"""
patch_claude_binary.py — Version-Independent Structural Binary Patcher for Claude Code (`ccide`).

Automatically adapts across Claude Code upgrades and architectures (`macOS Mach-O` & `Linux ELF`)
without hardcoded file offsets or hardcoded minified variable names:

1. Dynamic `__BUN` Standalone Trailer Parser (`\\n---- Bun! ----\\n`):
   Reads the 32-byte Bun trailer header (`byte_count`, `mod_off`, `mod_len`) dynamically on any
   macOS or Linux Bun standalone binary, scans only JS source modules (`/\\$bunfs/root/*.js`),
   and clears the `StandaloneModuleGraph` bytecode cache flags for modified modules.
2. Unminified Invariant Anchor & Regex Matching:
   Resolves internal functions (`enterView`, `exitView`, `adoptAgent`) and component closures
   via invariant string literals (`"tengu_transcript_view_enter"`, `"tengu_transcript_view_exit"`,
   `"(resumed agent)"`, `"local_workflow"`, `".evictTerminal("`, `"PromptHintPart"`, `key:"mode"`).
3. Zero-Minifier-Dependency `process.pid` IPC:
   Communicates with `ccide-core sidebar` via `/tmp/.cc-ide/{req,cur,sb}_pid_<process.pid>.*`
   over `SIGUSR2` (`< 0.05ms` latency, zero background timer polling).

CLI Flags:
  --test                Patch into `/tmp/claude-test-bin` without modifying installed binary
  --verify              Dry-run in memory, print JSON compatibility report, exit 1 if core patches fail
  --binary <path>       Explicit path to a `claude.exe` / `claude` binary or `cli.js` bundle to inspect/patch
"""
import argparse
import glob
import json
import os
import re
import shutil
import struct
import subprocess
import sys
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


def discover_claude_bin() -> str:
    env_bin = os.environ.get("CCIDE_CLAUDE_BIN", "").strip()
    if env_bin and os.path.exists(env_bin):
        return env_bin
    which_claude = shutil.which("claude")
    if which_claude:
        real_claude = os.path.realpath(which_claude)
        candidate_exe = os.path.join(os.path.dirname(real_claude), "bin", "claude.exe")
        if os.path.exists(candidate_exe):
            return candidate_exe
        if os.path.exists(real_claude):
            return real_claude
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


def parse_bun_layout(buf: bytes | bytearray) -> BunLayout | None:
    """
    Dynamically locates Bun's universal `\\n---- Bun! ----\\n` trailer (present on macOS Mach-O
    and Linux ELF standalone binaries) and parses the `StandaloneModuleGraph` table.
    """
    marker = b"\n---- Bun! ----\n"
    tail_search_start = max(0, len(buf) - 32 * 1024 * 1024)
    mpos = buf.rfind(marker, tail_search_start)
    if mpos == -1:
        mpos = buf.rfind(marker)
    if mpos == -1 or mpos < 32:
        return None

    byte_count, mod_off, mod_len = struct.unpack_from("<QII", buf, mpos - 32)
    base = mpos - 32 - byte_count
    table_start = base + mod_off
    if base < 0 or mod_len == 0 or mod_len % 52 != 0 or table_start + mod_len > len(buf):
        return None

    n_mods = mod_len // 52
    js_modules: list[JsModuleSpan] = []
    for i in range(n_mods):
        entry_off = table_start + i * 52
        entry = struct.unpack_from("<13I", buf, entry_off)
        name_s, name_l, code_s, code_l = entry[0], entry[1], entry[2], entry[3]
        if base + name_s + name_l > len(buf) or base + code_s + code_l > len(buf):
            continue
        name = bytes(buf[base + name_s : base + name_s + name_l]).decode("utf-8", errors="ignore")
        if name.startswith("/$bunfs/") and code_l > 0:
            js_modules.append(JsModuleSpan(i, name, base + code_s, base + code_s + code_l))

    if not js_modules:
        return None
    return BunLayout(base=base, table_start=table_start, n_mods=n_mods, js_modules=js_modules)


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


def discover_runtime_symbols(buf: bytearray, layout: BunLayout | None) -> dict[str, str]:
    """
    Dynamically extracts minified symbol names for `enterTranscriptView`, `exitTranscriptView`,
    and `adoptCompletedSubagent` using unminified telemetry/fallback string literals.
    """
    re_enter = re.compile(
        rb'function\s+([A-Za-z0-9_$]+)\s*\([^,)]+,[^,)]+\)\s*\{[^}]{0,200}["\']tengu_transcript_view_enter["\']'
    )
    re_exit = re.compile(
        rb'function\s+([A-Za-z0-9_$]+)\s*\([^,)]+\)\s*\{[^}]{0,200}["\']tengu_transcript_view_exit["\']'
    )
    re_adopt = re.compile(
        rb'function\s+([A-Za-z0-9_$]+)\s*\([A-Za-z0-9_$]+,[A-Za-z0-9_$]+\)\s*\{[^}]{0,260}["\']local_agent["\'][^}]{0,200}["\']\(resumed agent\)["\']'
    )

    syms: dict[str, str] = {}
    for _, region in iter_js_regions(buf, layout):
        blob = bytes(region)
        if "enter_fn" not in syms:
            m = re_enter.search(blob)
            if m:
                syms["enter_fn"] = m.group(1).decode("ascii")
        if "exit_fn" not in syms:
            m = re_exit.search(blob)
            if m:
                syms["exit_fn"] = m.group(1).decode("ascii")
        if "adopt_fn" not in syms:
            m = re_adopt.search(blob)
            if m:
                syms["adopt_fn"] = m.group(1).decode("ascii")
        if len(syms) == 3:
            break
    return syms


def apply_regex_patch_in_regions(
    buf: bytearray,
    layout: BunLayout | None,
    pattern: re.Pattern[bytes],
    builder,
    label: str,
) -> tuple[int | None, str]:
    for base_off, region in iter_js_regions(buf, layout):
        blob = bytes(region)
        m = pattern.search(blob)
        if not m:
            continue
        old_bytes = m.group(0)
        new_bytes = builder(m)
        if len(new_bytes) > len(old_bytes):
            return (
                None,
                f"[{label}] replacement exceeds original span ({len(new_bytes)} > {len(old_bytes)} bytes)",
            )
        padded = new_bytes + b" " * (len(old_bytes) - len(new_bytes))
        abs_pos = base_off + m.start()
        buf[abs_pos : abs_pos + len(old_bytes)] = padded
        return abs_pos, f"[OK] {label} @ offset {abs_pos} ({len(new_bytes)}/{len(old_bytes)} bytes)"
    return None, f"[MISS] {label}: pattern not found"


def _detect_js_syntax_cmd() -> list[str] | None:
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


def patch_buffer_in_memory(buf: bytearray) -> dict:
    """
    Applies all structural patches to `buf` in place and returns a structured report.
    """
    layout = parse_bun_layout(buf)
    orig_mod_snapshots: dict[int, bytes] = {}
    if layout is not None:
        for mod in layout.js_modules:
            orig_mod_snapshots[mod.mod_idx] = bytes(buf[mod.src_start : mod.src_end])
    else:
        orig_mod_snapshots[-1] = bytes(buf)
    syms = discover_runtime_symbols(buf, layout)
    results: list[dict] = []
    patched_offsets: list[int] = []

    def record(name: str, required: bool, pos: int | None, msg: str):
        if pos is not None:
            patched_offsets.append(pos)
        results.append(
            {
                "name": name,
                "required": required,
                "applied": pos is not None,
                "offset": pos,
                "message": msg,
            }
        )

    # 1. Subagent retention timers (`var X=3000,Y=30000,Z=30000;` before `in_process_teammate`)
    re_ret1 = re.compile(
        rb"var\s+([A-Za-z0-9_$]+)=\d+,([A-Za-z0-9_$]+)=30000,([A-Za-z0-9_$]+)=30000;"
        rb"(?=function\s+[A-Za-z0-9_$]+\([^)]*\)\{[^}]{0,220}in_process_teammate)"
    )
    pos, msg = apply_regex_patch_in_regions(
        buf,
        layout,
        re_ret1,
        lambda m: b"var " + m.group(1) + b"=3000," + m.group(2) + b"=9e9," + m.group(3) + b"=9e9;",
        "Subagent retention timers (9e9 ms)",
    )
    record("subagent_retention_timers", True, pos, msg)

    # 2. Transcript view retention timer (`var X=3000;var Y=30000;` before `local_agent`)
    re_ret2 = re.compile(
        rb"var\s+([A-Za-z0-9_$]+)=3000;\s*var\s+([A-Za-z0-9_$]+)=30000;"
        rb"(?=function\s+[A-Za-z0-9_$]+\([^)]*\)\{[^}]{0,180}local_agent)"
    )
    pos, msg = apply_regex_patch_in_regions(
        buf,
        layout,
        re_ret2,
        lambda m: b"var " + m.group(1) + b"=3000;var " + m.group(2) + b"=9e9;",
        "Transcript view exit retention timer (9e9 ms)",
    )
    record("transcript_view_retention", True, pos, msg)

    # 3. Never hide completed background subagents (`function aD(h){...quietlyParked...}`)
    re_vis = re.compile(
        rb"function\s+([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\)\{"
        rb'return\s+\2\.type==="local_agent"&&\2\.status==="completed"&&[^}]{15,260}quietlyParked[^}]+\}'
    )
    pos, msg = apply_regex_patch_in_regions(
        buf,
        layout,
        re_vis,
        lambda m: b"function " + m.group(1) + b"(" + m.group(2) + b"){return!1}",
        "Keep completed background subagents visible",
    )
    record("keep_completed_visible", True, pos, msg)

    # 4. Event-driven SIGUSR2 native view-switch & state push hook inside `TQ` task tick
    enter_fn = syms.get("enter_fn", "")
    exit_fn = syms.get("exit_fn", "")
    adopt_fn = syms.get("adopt_fn", "")

    if enter_fn and exit_fn and adopt_fn:
        # Locate enclosing `TQ` model variable (`_e=Bu()`) if present in the same module
        re_model_var = re.compile(
            rb"showWorkflows:[A-Za-z0-9_$]+\}=[A-Za-z0-9_$]+,[A-Za-z0-9_$]+=[^;]{1,80}?([A-Za-z0-9_$]+)=[A-Za-z0-9_$]+\(\)"
        )
        model_expr = b'""'
        for _, reg in iter_js_regions(buf, layout):
            b_reg = bytes(reg)
            if b".evictTerminal(" in b_reg and b'"local_workflow"' in b_reg:
                mm = re_model_var.search(b_reg)
                if mm:
                    model_expr = mm.group(1) + b'??""'
                break

        re_jn = re.compile(
            rb"([A-Za-z0-9_$]+)=\(\)=>\{let\s+([A-Za-z0-9_$]+)=Date\.now\(\);"
            rb"let\s+([A-Za-z0-9_$]+)=!1;"
            rb"(?:let\s+[^;]{1,80};){1,10}"
            rb"let\s+[A-Za-z0-9_$]+=[A-Za-z0-9_$]+\(([A-Za-z0-9_$]+),([A-Za-z0-9_$]+)\);"
            rb'for\(const\s+[A-Za-z0-9_$]+\s+of\s+Object\.values\(\4\)\)\{if\([A-Za-z0-9_$]+\.type==="local_workflow"\)\{'
            rb'if\([A-Za-z0-9_$]+\.status!=="running"&&\([A-Za-z0-9_$]+\.evictAfter\?\?Infinity\)<=\2\)([A-Za-z0-9_$]+)\.evictTerminal\([A-Za-z0-9_$]+\.id\);continue\}'
            rb"if\(!([A-Za-z0-9_$]+)\([A-Za-z0-9_$]+\)\)\{continue\}"
            rb".{120,1800}?"
            rb"if\([A-Za-z0-9_$]+\.length>0\|\|[A-Za-z0-9_$]+\.length>0\|\|[A-Za-z0-9_$]+\.length>0\)([A-Za-z0-9_$]+)\(\([A-Za-z0-9_$]+\)=>\{.{80,1000}?\}\);"
            rb"if\(\3&&!([A-Za-z0-9_$]+)\)([A-Za-z0-9_$]+)\(\2\)\}"
            rb"(,[^;]{10,180};else\s+\1=[^;]{3,40};[A-Za-z0-9_$]+\(\1,[^;]{3,50}\);)"
        )

        def build_jn(m: re.Match[bytes]) -> bytes:
            jn_v = m.group(1)
            now_v = m.group(2)
            lo_v = m.group(3)
            te_v = m.group(4)
            lt_v = m.group(5)
            to_v = m.group(6)
            um_v = m.group(7)
            qt_v = m.group(8)
            ge_v = m.group(9)
            hn_v = m.group(10)
            tail = m.group(11)

            hook = (
                jn_v
                + b"=()=>{let "
                + now_v
                + b"=Date.now(),"
                + lo_v
                + b"=!1;for(const u of Object.values("
                + te_v
                + b")){if("
                + um_v
                + b'(u)&&u.status==="running"&&!u.isIdle)'
                + lo_v
                + b"=!0}if("
                + lo_v
                + b"&&!"
                + ge_v
                + b")"
                + hn_v
                + b"("
                + now_v
                + b")}"
                + tail
                + b'try{let fs=import.meta.require("fs"),p=process.pid,g=globalThis,d="/tmp/.cc-ide";'
                + b"g.__cc={Qt:"
                + qt_v
                + b",to:"
                + to_v
                + b",te:"
                + te_v
                + b",lt:"
                + lt_v
                + b",fs};"
                + b"if(!g.__cc_h){fs.mkdirSync(d,{recursive:!0});"
                + b"g.__cc_h=()=>{try{let c=g.__cc;if(!c)return;"
                + b"let rf=`${d}/req_pid_${p}.json`;if(!c.fs.existsSync(rf))return;"
                + b'let q=JSON.parse(c.fs.readFileSync(rf,"utf8"));c.fs.unlinkSync(rf);'
                + b'if(!q.id||q.id==="main"){if(c.lt!==void 0)'
                + exit_fn.encode("ascii")
                + b"(c.Qt)}else{if(!c.te[q.id])"
                + adopt_fn.encode("ascii")
                + b'({agentId:q.id,description:q.desc||"subagent",agentType:q.type||"general-purpose"},c.to);'
                + enter_fn.encode("ascii")
                + b'(q.id,c.Qt)}}catch{}};process.on("SIGUSR2",g.__cc_h)}'
                + b"let st=`${"
                + lt_v
                + b'??"main"}\\n${'
                + model_expr
                + b"}\\n${Object.keys("
                + te_v
                + b").length}`;if(g.__cc_s!==st){g.__cc_s=st;fs.writeFileSync(`${d}/cur_pid_${p}.txt`,st);"
                + b'let sp=`${d}/sb_pid_${p}.pid`;if(fs.existsSync(sp)){let k=+fs.readFileSync(sp,"utf8");if(k>1)process.kill(k,"SIGUSR2")}}}catch{}'
            )
            return hook

        pos, msg = apply_regex_patch_in_regions(
            buf,
            layout,
            re_jn,
            build_jn,
            f"Event-driven SIGUSR2 view-switch hook ({enter_fn}/{exit_fn}/{adopt_fn})",
        )
        record("sigusr2_view_switch_hook", True, pos, msg)
    else:
        record(
            "sigusr2_view_switch_hook",
            True,
            None,
            f"[MISS] Runtime symbols incomplete: {syms}",
        )

    # 5a. Hide built-in bottom `⏺ main / ◯ subagent` footer task list (`TQ` early return)
    re_tq_hide = re.compile(
        rb"if\(([A-Za-z0-9_$]+)\|\|([A-Za-z0-9_$]+\.length===0&&![A-Za-z0-9_$]+&&![A-Za-z0-9_$]+)\)\{return null\}"
    )
    pos, msg = apply_regex_patch_in_regions(
        buf,
        layout,
        re_tq_hide,
        lambda m: b"if(!0||" + m.group(2) + b"){return null}",
        "Hide built-in bottom subagent footer list",
    )
    record("hide_footer_subagent_list", False, pos, msg)

    # 5b. Report 0 visible footer task rows (`Uk()`)
    re_uk = re.compile(
        rb"(function\s+[A-Za-z0-9_$]+\(\)\{let\[[A-Za-z0-9_$]+\]=[A-Za-z0-9_$]+\(([A-Za-z0-9_$]+)\),[A-Za-z0-9_$]+=[A-Za-z0-9_$]+\([A-Za-z0-9_$]+\);return\s+)"
        rb"[A-Za-z0-9_$]+\(\)"
        rb"(\?0:[A-Za-z0-9_$]+\})(?=function\s+\2\(\)\{[^}]{0,220}taskDecorations)"
    )
    pos, msg = apply_regex_patch_in_regions(
        buf,
        layout,
        re_uk,
        lambda m: m.group(1) + b"!0" + m.group(3),
        "Report 0 visible footer task rows",
    )
    record("zero_footer_task_rows", False, pos, msg)

    # 6a. Hide statusline mode pill (`key:"mode"`)
    re_mode_pill = re.compile(
        rb'function\s+([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\)\{return\{key:"mode",width:.{5,80}?,"mode"\)\}\}'
    )
    pos, msg = apply_regex_patch_in_regions(
        buf,
        layout,
        re_mode_pill,
        lambda m: b"function " + m.group(1) + b"(" + m.group(2) + b"){return null}",
        "Hide statusline mode pill",
    )
    record("hide_mode_pill", False, pos, msg)

    # 6b. Hide bottom `⏵⏵ bypass permissions on` hint row (`PromptHintPart`) unless interactive
    re_dfe = re.compile(
        rb"(var\s+[A-Za-z0-9_$]+=\([A-Za-z0-9_$]+\)=>\{let\s+[A-Za-z0-9_$]+=[A-Za-z0-9_$]+\(\d+\),([A-Za-z0-9_$]+),([A-Za-z0-9_$]+);[^;]+;else\s+\2=[^;]+;"
        rb"const\s+([A-Za-z0-9_$]+)=!\2\.isInputEmpty,([A-Za-z0-9_$]+)=\2\.isSearching\|\|\2\.exitMessage\.show\|\|[^;]+showExpandPasteHint[^;]+;)"
        rb"let\s+([A-Za-z0-9_$]+);if\([^)]+\)\6=([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+),\{\.\.\.\2\}\),[^;]+;else\s+\6=[^;]+;"
        rb"let\s+[A-Za-z0-9_$]+;if\([^)]+\)[A-Za-z0-9_$]+=\7\(([A-Za-z0-9_$]+\.PromptHintPart),\{([^}]+)\}\),[^;]+;else\s+[A-Za-z0-9_$]+=[^;]+;return\s+[A-Za-z0-9_$]+\};"
    )
    pos, msg = apply_regex_patch_in_regions(
        buf,
        layout,
        re_dfe,
        lambda m: (
            m.group(1)
            + b"let "
            + m.group(6)
            + b"="
            + m.group(5)
            + b"?"
            + m.group(7)
            + b"("
            + m.group(8)
            + b",{..."
            + m.group(2)
            + b"}):null;return "
            + m.group(5)
            + b"?"
            + m.group(7)
            + b"("
            + m.group(9)
            + b",{"
            + m.group(10)
            + b"}):null};"
        ),
        "Hide bottom bypass-permissions row while keeping interactive hints",
    )
    record("hide_bypass_permissions_row", False, pos, msg)

    cleared_modules = clear_bun_bytecode_for_offsets(buf, layout, patched_offsets)

    # Syntax-check every modified JS module when `node` can parse the baseline module
    syntax_errors: list[str] = []
    js_check_cmd = _detect_js_syntax_cmd()
    if js_check_cmd is not None and patched_offsets:
        if layout is not None and cleared_modules:
            cleared_idx_set = {idx for idx, _ in cleared_modules}
            for mod in layout.js_modules:
                if mod.mod_idx in cleared_idx_set:
                    orig_bytes = orig_mod_snapshots.get(mod.mod_idx, b"")
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
            orig_bytes = orig_mod_snapshots.get(-1, b"")
            if subprocess.run(js_check_cmd, input=orig_bytes, capture_output=True).returncode == 0:
                proc = subprocess.run(js_check_cmd, input=bytes(buf), capture_output=True)
                if proc.returncode != 0:
                    err_txt = proc.stderr.decode("utf-8", errors="replace").strip()
                    syntax_errors.append(f"Bundle: {err_txt}")

    required_ok = all(r["applied"] for r in results if r["required"]) and not syntax_errors

    return {
        "bun_standalone": layout is not None,
        "bun_modules": layout.n_mods if layout else 0,
        "js_modules": len(layout.js_modules) if layout else 0,
        "symbols": syms,
        "required_ok": required_ok,
        "syntax_errors": syntax_errors,
        "patches": results,
        "cleared_bytecode_modules": [
            {"index": idx, "name": nm} for idx, nm in cleared_modules
        ],
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
    os.chmod(tmp_dst, 0o755)
    if sys.platform == "darwin" and shutil.which("codesign"):
        subprocess.run(["codesign", "-f", "-s", "-", tmp_dst], check=True)
    os.replace(tmp_dst, dst_path)
    print(f"  [OK] Installed -> {dst_path}")
    return report


def main():
    parser = argparse.ArgumentParser(description="Structural dynamic patcher for Claude Code")
    parser.add_argument("--test", action="store_true", help="Write patched binary to /tmp/claude-test-bin")
    parser.add_argument("--verify", action="store_true", help="Dry-run in memory and output JSON report")
    parser.add_argument("--binary", type=str, default="", help="Explicit path to claude binary / cli.js")
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
    is_already_patched = b"/tmp/.cc-ide" in cur_bytes or b"g.__cc_h" in cur_bytes
    if not os.path.exists(bak_path) or not is_already_patched:
        if is_already_patched and not os.path.exists(bak_path):
            print(
                f"  [WARN] {target_bin} is already patched and has no .bak; reinstall @anthropic-ai/claude-code to refresh .bak."
            )
            sys.exit(0)
        shutil.copy2(target_bin, bak_path)
        print(f"  [OK] Created/refreshed pristine backup -> {bak_path}")

    dst = "/tmp/claude-test-bin" if args.test else target_bin
    try:
        apply_patches(bak_path, dst)
        res = subprocess.run([dst, "--version"], capture_output=True, text=True, check=True)
        print(f"  [OK] Verified `{dst} --version` -> {res.stdout.strip()}")
    except RuntimeError as e:
        print(f"  [WARN] Binary patch skipped (Claude Code layout changed): {e}")
        sys.exit(0)


if __name__ == "__main__":
    main()
