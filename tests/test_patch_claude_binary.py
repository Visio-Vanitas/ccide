#!/usr/bin/env python3
"""Unit tests for scripts/patch_claude_binary.py (stdlib ``unittest`` only).

Run from the repository root with::

    python3 -m unittest discover -s tests -v

These tests never touch an installed Claude Code binary: every fixture is an
in-memory synthetic buffer built here.
"""

import os
import struct
import sys
import unittest

_SCRIPTS_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "scripts")
sys.path.insert(0, os.path.abspath(_SCRIPTS_DIR))

import patch_claude_binary as patcher  # type: ignore[import-not-found]  # noqa: E402

BUN_MARKER = b"\n---- Bun! ----\n"
BUN_TRAILER_HEADER_SIZE = 32
BUN_ENTRY_SIZE = 52
BUN_NAME_PREFIX = b"/$bunfs/root/"


# ---------------------------------------------------------------------------
# Synthetic (re-minified) Claude Code JS fixture
#
# Deliberately uses identifiers that do NOT appear in the real bundle so the
# regexes have to resolve them structurally rather than by hardcoded name.
# ---------------------------------------------------------------------------

JN_FILLER_A = b"/*" + b"y" * 700 + b"*/"
JN_FILLER_B = b"w.tick();" * 70  # 700 chars, no '}' so the lazy span is exact


def build_js_bundle(include_optional_ui=True):
    """Returns a re-minified JS bundle exercising every patch pattern."""
    parts = []

    # 1. Subagent retention timers, immediately before `in_process_teammate`.
    parts.append(
        b"var rT=3000,rU=30000,rV=30000;"
        b'function tickLoop(a){let z="in_process_teammate";return z}'
    )

    # 2. Transcript view retention timer before `local_agent`.
    parts.append(
        b"var rW=3000;var rX=30000;"
        b'function fnAdoptZ(a,b){let t="local_agent",d="(resumed agent)";return [t,d]}'
    )

    # 3. Completed-background-subagent visibility predicate.
    parts.append(
        b"function hideDone(m){"
        b'return m.type==="local_agent"&&m.status==="completed"&&m.someFlagField===!0&&'
        b"m.quietlyParked===!0&&m.zz!==1}"
    )

    # 4. Runtime symbol anchors (dynamic minified names).
    parts.append(
        b'function fnEnter42(q,w){let a="tengu_transcript_view_enter";return a}'
        b'function fnExit9(q){let a="tengu_transcript_view_exit";return a}'
    )

    # 4b. Event-driven task tick (`re_jn`).
    parts.append(
        b"var tickLoop,moreDataPayload=0;"
        b"if(condTick)tickLoop=()=>{let nA=Date.now();let lB=!1;"
        b"let x1=foo();let x2=bar();let mS=getModels(mA,mB);"
        b'for(const ent of Object.values(mA)){if(ent.type==="local_workflow"){'
        b'if(ent.status!=="running"&&(ent.evictAfter??Infinity)<=nA)'
        b"evictor.evictTerminal(ent.id);continue}"
        b"if(!checker(ent)){continue}"
        + JN_FILLER_A
        + b"if(a1.length>0||a2.length>0||a3.length>0)pusher((v)=>{"
        + JN_FILLER_B
        + b"return v});"
        + b"if(lB&&!guard)err(nA)}"
        + b",moreDataPayload=1;else tickLoop=queuedTick;enqueue(tickLoop,cbOne);"
    )

    # 5a. Bottom footer task list early return.
    parts.append(b"function footerList(){if(qA||rows.length===0&&!qB&&!qC){return null}return rows}")

    # 5b. Visible footer task row counter (`Uk()`), whose lookahead resolves the
    # model accessor `stateVar` that decorates task rows.
    parts.append(
        b"function pillFn(){let[rows]=useStore(stateVar),b=foo(x);return hasRows()?0:empty}"
        b'function stateVar(){let t="taskDecorations";return t}'
    )

    if include_optional_ui:
        # 6a. Statusline mode pill.
        parts.append(
            b'function pillBadge(q){return{key:"mode",width:getW(q),node:el(Badge,{mode:q},"mode")}}'
        )

    # 6b. Bypass-permissions hint row.
    parts.append(
        b"var hintComp=(arg)=>{let q=soma(4),pp,cc;if(zz)pp=arg;else pp=two;"
        b"const aa=!pp.isInputEmpty,"
        b"bb=pp.isSearching||pp.exitMessage.show||zz.showExpandPasteHint||ww;"
        b"let hv;if(condOne)hv=prettyFn(Inner,{...pp}),tailOne=1;else hv=otherOne;"
        b"let gh;if(condTwo)gh=prettyFn(ns.PromptHintPart,{innerOne:1}),tailTwo=2;"
        b"else gh=otherTwo;return gh};"
    )

    return b"".join(parts)


def build_bun_standalone(js0, js1=b"// second chunk untouched\n", js2=b"// third chunk untouched\n"):
    """Builds a synthetic `__BUN` standalone buffer.

    Layout (offsets stored in the module table are relative to `base`, which is
    the start of the standalone payload right after a fake executable prefix)::

        [ 4096-byte fake Mach-O prefix ][ js0 ][ js1 ][ js2 ][ names ][ table ][ <QII ][ marker ]

    Every entry claims nonzero bytecode offset/length and the bytecode cache
    flag `0x100` so clearing can be observed.
    """
    names = [
        BUN_NAME_PREFIX + b"chunk-one.js",
        BUN_NAME_PREFIX + b"chunk-two.js",
        BUN_NAME_PREFIX + b"chunk-three.js",
    ]
    code_blobs = [js0, js1, js2]

    payload = bytearray()
    code_offsets = []
    for blob in code_blobs:
        code_offsets.append(len(payload))
        payload += blob
    name_offsets = []
    for name in names:
        name_offsets.append(len(payload))
        payload += name

    table_offset = len(payload)
    table = bytearray()
    for i in range(3):
        entry = [0] * 13
        entry[0] = name_offsets[i]
        entry[1] = len(names[i])
        entry[2] = code_offsets[i]
        entry[3] = len(code_blobs[i])
        entry[6] = 0x9000 + i * 0x100  # bytecode offset (nonzero)
        entry[7] = 0x400 + i  # bytecode length (nonzero)
        entry[12] = 0x100 | 0x1  # bytecode cache flag
        table += struct.pack("<13I", *entry)
    payload += table

    # `byte_count` is the distance from `base` to the trailer header start.
    # The trailer is a 32-byte header region (a 16-byte `<QII` struct followed
    # by 16 bytes of padding) immediately preceding the 16-byte marker.
    byte_count = len(payload)
    header = struct.pack("<QII", byte_count, table_offset, len(table))
    payload += header + b"\x00" * (BUN_TRAILER_HEADER_SIZE - 16) + BUN_MARKER

    prefix = b"\x00" * 4096
    buf = bytearray(prefix + payload)

    base = len(prefix)
    table_start = base + table_offset
    return buf, base, table_start


def read_table_entry(buf, table_start, idx):
    return struct.unpack_from("<13I", buf, table_start + idx * BUN_ENTRY_SIZE)


REQUIRED_PATCH_NAMES = {
    "subagent_retention_timers",
    "transcript_view_retention",
    "keep_completed_visible",
    "sigusr2_view_switch_hook",
}


class BunStandaloneMutationTests(unittest.TestCase):
    """(a) Synthetic `__BUN` trailer + re-minified bundle."""

    @classmethod
    def setUpClass(cls):
        cls.js0 = build_js_bundle()
        cls.buf, cls.base, cls.table_start = build_bun_standalone(cls.js0)
        cls.original_len = len(cls.buf)
        cls.before = [read_table_entry(cls.buf, cls.table_start, i) for i in range(3)]
        cls.report = patcher.patch_buffer_in_memory(cls.buf)

    def test_layout_is_detected(self):
        layout = patcher.parse_bun_layout(self.buf)
        self.assertIsNotNone(layout)
        self.assertEqual(layout.base, self.base)
        self.assertEqual(layout.n_mods, 3)
        self.assertEqual(len(layout.js_modules), 3)

    def test_byte_length_is_preserved_exactly(self):
        self.assertEqual(len(self.buf), self.original_len)

    def test_report_is_bun_standalone(self):
        self.assertIs(self.report["bun_standalone"], True)
        self.assertEqual(self.report["bun_modules"], 3)
        self.assertEqual(self.report["js_modules"], 3)

    def test_symbols_are_resolved_without_hardcoded_names(self):
        self.assertEqual(
            self.report["symbols"],
            {
                "enter_fn": "fnEnter42",
                "exit_fn": "fnExit9",
                "adopt_fn": "fnAdoptZ",
            },
        )

    def test_all_patches_applied_and_required_ok(self):
        self.assertEqual(len(self.report["patches"]), 8)
        for patch in self.report["patches"]:
            self.assertTrue(patch["applied"], msg=f"{patch['name']}: {patch['message']}")
        self.assertIs(self.report["required_ok"], True)
        self.assertTrue(
            REQUIRED_PATCH_NAMES.issubset({p["name"] for p in self.report["patches"]})
        )

    def test_modified_module_bytecode_cache_is_cleared(self):
        entry = read_table_entry(self.buf, self.table_start, 0)
        self.assertEqual(entry[6], 0)
        self.assertEqual(entry[7], 0)
        self.assertEqual(entry[12] & 0x100, 0)
        # The rest of the flag word is preserved.
        self.assertEqual(entry[12] & 0x1, 0x1)
        cleared = self.report["cleared_bytecode_modules"]
        self.assertEqual([m["index"] for m in cleared], [0])
        self.assertEqual(cleared[0]["name"], "/$bunfs/root/chunk-one.js")

    def test_untouched_modules_keep_bytecode_cache(self):
        for idx in (1, 2):
            entry = read_table_entry(self.buf, self.table_start, idx)
            self.assertEqual(entry[6], self.before[idx][6])
            self.assertEqual(entry[7], self.before[idx][7])
            self.assertNotEqual(entry[6], 0)
            self.assertNotEqual(entry[7], 0)
            self.assertEqual(entry[12] & 0x100, 0x100)

    def test_original_js_source_is_otherwise_preserved(self):
        layout = patcher.parse_bun_layout(self.buf)
        module0 = layout.js_modules[0]
        patched_src = bytes(self.buf[module0.src_start : module0.src_end])
        self.assertEqual(len(patched_src), len(self.js0))
        # Non-patched spans survive byte-for-byte.
        self.assertIn(b'function hideDone(m){return!1}', patched_src)
        self.assertIn(b"var rT=3000,rU=9e9,rV=9e9;", patched_src)
        self.assertIn(b'g.__cc_h', patched_src)


class PlainJsFallbackTests(unittest.TestCase):
    """(b) Plain `cli.js` bundle without a Bun trailer."""

    def test_plain_bundle_patches_without_bun_layout(self):
        buf = bytearray(build_js_bundle())
        original_len = len(buf)
        self.assertIsNone(patcher.parse_bun_layout(buf))

        report = patcher.patch_buffer_in_memory(buf)

        self.assertIs(report["bun_standalone"], False)
        self.assertEqual(report["bun_modules"], 0)
        self.assertEqual(report["js_modules"], 0)
        self.assertEqual(len(buf), original_len)
        self.assertIs(report["required_ok"], True)
        for patch in report["patches"]:
            self.assertTrue(patch["applied"], msg=f"{patch['name']}: {patch['message']}")
        self.assertEqual(report["cleared_bytecode_modules"], [])
        self.assertIn(b'g.__cc_h', bytes(buf))


class FutureLayoutDriftTests(unittest.TestCase):
    """(c) Optional UI patches may drift without breaking required patches."""

    def test_missing_optional_patch_is_graceful(self):
        buf = bytearray(build_js_bundle(include_optional_ui=False))
        report = patcher.patch_buffer_in_memory(buf)

        patches = {p["name"]: p for p in report["patches"]}
        self.assertIs(patches["hide_mode_pill"]["applied"], False)
        self.assertIs(patches["hide_mode_pill"]["required"], False)
        self.assertIs(report["required_ok"], True)

        for name in REQUIRED_PATCH_NAMES:
            self.assertTrue(patches[name]["applied"], msg=name)
        # Other optional patches still apply when present.
        self.assertTrue(patches["hide_footer_subagent_list"]["applied"])
        self.assertTrue(patches["zero_footer_task_rows"]["applied"])
        self.assertTrue(patches["hide_bypass_permissions_row"]["applied"])

    def test_missing_required_patch_marks_report_not_ok(self):
        # A bundle with no runtime anchors at all: required patches must fail.
        buf = bytearray(b"const nothing = 1;\n")
        report = patcher.patch_buffer_in_memory(buf)
        self.assertIs(report["required_ok"], False)
        self.assertIsNone(patcher.parse_bun_layout(buf))
        required = {p["name"]: p for p in report["patches"] if p["required"]}
        self.assertEqual(set(required), REQUIRED_PATCH_NAMES)
        for patch in required.values():
            self.assertIs(patch["applied"], False)


class BunLayoutParserEdgeCaseTests(unittest.TestCase):
    def test_returns_none_without_trailer(self):
        self.assertIsNone(patcher.parse_bun_layout(bytearray(b"plain js only\n")))

    def test_returns_none_for_truncated_trailer(self):
        buf = bytearray(b"\x00" * 16 + BUN_MARKER)
        self.assertIsNone(patcher.parse_bun_layout(buf))

    def test_returns_none_when_graph_has_no_js_modules(self):
        # Valid trailer, but a single module whose name is not under /$bunfs/.
        payload = bytearray(b"module code")
        name_off = len(payload)
        payload += b"/not-bunfs/root/x.js"
        table_off = len(payload)
        entry = [0] * 13
        entry[0] = name_off
        entry[1] = len(b"/not-bunfs/root/x.js")
        entry[2] = 0
        entry[3] = len(b"module code")
        payload += struct.pack("<13I", *entry)
        payload += struct.pack("<QII", len(payload), table_off, BUN_ENTRY_SIZE)
        payload += BUN_MARKER

        self.assertIsNone(patcher.parse_bun_layout(bytearray(b"\x00" * 64 + payload)))

    def test_parse_does_not_mutate(self):
        buf, _, _ = build_bun_standalone(build_js_bundle())
        snapshot = bytes(buf)
        patcher.parse_bun_layout(buf)
        self.assertEqual(bytes(buf), snapshot)

    def test_synthetic_bundle_passes_v8_syntax_check(self):
        js_cmd = patcher._detect_js_syntax_cmd()
        if js_cmd is None:
            self.skipTest("node not installed")
        buf, _, _ = build_bun_standalone(build_js_bundle())
        report = patcher.patch_buffer_in_memory(buf)
        self.assertEqual(report["syntax_errors"], [])
        self.assertIs(report["required_ok"], True)

    def test_claude_code_2_1_323_zustand_selector_and_ui_drift(self):
        """Regression test for Issue #7 (Claude Code 2.1.322+ boolean trap & UI drift)."""
        parts = [
            b"var rT=3000,rU=30000,rV=30000;"
            b'function tickLoop(a){let z="in_process_teammate";return z}',
            b"var rW=3000;var rX=30000;"
            b'function fnAdoptZ(a,b){let t="local_agent",d="(resumed agent)";return [t,d]}',
            b"function hideDone(m){"
            b'return m.type==="local_agent"&&m.status==="completed"&&m.someFlagField===!0&&'
            b"m.quietlyParked===!0&&m.zz!==1}",
            b'function fnEnter42(q,w){let a="tengu_transcript_view_enter";return a}'
            b'function fnExit9(q){let a="tengu_transcript_view_exit";return a}',
            # 2.1.322+ footer component header where `Ie=nt()` returns boolean `false`:
            b"function TQ(M){let{showWorkflows:H}=M,ee=H===void 0?!1:H,"
            b"xe=useZustandStore(hGt),Ie=nt(),$e=zTe();"
            b"var tickLoop,moreDataPayload=0;"
            b"if(condTick)tickLoop=()=>{let nA=Date.now();let lB=!1;"
            b"let x1=foo();let mS=getModels(xe,mB);"
            b'for(const ent of Object.values(xe)){if(ent.type==="local_workflow"){'
            b'if(ent.status!=="running"&&(ent.evictAfter??Infinity)<=nA)'
            b"evictor.evictTerminal(ent.id);continue}"
            b"if(!checker(ent)){continue}"
            + JN_FILLER_A
            + b"if(a1.length>0||a2.length>0||a3.length>0)pusher((v)=>{"
            + JN_FILLER_B
            + b"return v});"
            + b"if(lB&&!guard)err(nA)}"
            + b",moreDataPayload=1;else tickLoop=queuedTick;enqueue(tickLoop,cbOne);"
            + b"return null}",
            b"function footerList(){if(qA||rows.length===0&&!qB&&!qC){return null}return rows}",
            # 2.1.323 `Uk()` lookahead uses `idleTeammatesExpanded` instead of `taskDecorations`
            b"function pillFn(){let[rows]=useStore(stateVar),b=foo(x);return hasRows()?0:empty}"
            b'function stateVar(){let t="idleTeammatesExpanded";return t}',
            b'function pillBadge(q){return{key:"mode",width:getW(q),node:el(Badge,{mode:q},"mode")}}',
            # 2.1.323 `PromptHintPart` spreads `{...pp,wrappedRowWidth:cc}`
            b"var hintComp=(arg)=>{let q=soma(4),pp,cc;if(zz)pp=arg;else pp=two;"
            b"const aa=!pp.isInputEmpty,"
            b"bb=pp.isSearching||pp.exitMessage.show||zz.showExpandPasteHint||ww;"
            b"let hv;if(condOne)hv=prettyFn(Inner,{...pp,wrappedRowWidth:cc}),tailOne=1;else hv=otherOne;"
            b"let gh;if(condTwo)gh=prettyFn(ns.PromptHintPart,{innerOne:1}),tailTwo=2;"
            b"else gh=otherTwo;return gh};",
        ]
        buf, _, _ = build_bun_standalone(b"".join(parts))
        report = patcher.patch_buffer_in_memory(buf)
        self.assertTrue(report["required_ok"])
        self.assertEqual(report["syntax_errors"], [])
        for p in report["patches"]:
            self.assertTrue(p["applied"], msg=f"{p['name']}: {p['message']}")
        patched_bytes = bytes(buf)
        self.assertIn(
            b"var __cm=useZustandStore((x)=>x.mainLoopModelForSession??x.mainLoopModel);",
            patched_bytes,
        )
        self.assertNotIn(b"${Ie??", patched_bytes)
        self.assertIn(b"{...pp,wrappedRowWidth:cc}", patched_bytes)


if __name__ == "__main__":
    unittest.main()
